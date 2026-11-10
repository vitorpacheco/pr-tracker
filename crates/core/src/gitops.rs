//! Manages local clones and per-PR worktrees.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::msg;
use crate::provider::{Client, Item};
use crate::toolchain::{self, Ctx};

/// Catalog key of the error for a worktree with uncommitted changes.
pub const ERR_DIRTY: &str = "worktree tem alterações locais";

const ERR_OTHER_BRANCH: &str = "o worktree está em outra branch; selecione %s antes de atualizar";

static UNSAFE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^A-Za-z0-9._-]+").expect("constant pattern"));

/// Runs git in `dir` and returns its trimmed stdout.
pub fn git(ctx: &Ctx, dir: &Path, args: &[&str]) -> Result<String> {
    run(ctx, dir, args)
}

/// [`git`] with arguments that may be paths, which are not always UTF-8.
fn run<S: AsRef<OsStr>>(ctx: &Ctx, dir: &Path, args: &[S]) -> Result<String> {
    let path = toolchain::lookup(ctx, "git").map_err(|err| msg!("localizando git: %w", err))?;
    let mut cmd = toolchain::command(ctx, &path);
    cmd.args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    let subcommand = args
        .first()
        .map(|arg| arg.as_ref().to_string_lossy())
        .unwrap_or_default();
    let output = match toolchain::output(ctx, &mut cmd) {
        Ok(output) => output,
        // Cancellation and deadlines stay recognisable to the caller.
        Err(err @ (Error::Canceled | Error::DeadlineExceeded)) => return Err(err),
        Err(err) => return Err(Error::external(format!("git {subcommand}: {err}"))),
    };
    if !output.status.success() {
        let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if message.is_empty() {
            message = output.status.to_string();
        }
        return Err(Error::external(format!("git {subcommand}: {message}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A short, stable identifier for a PR, used in worktree folder names: the
/// first eight hex characters of the SHA-256 of host, repository and number.
pub fn hash(pr: &Item) -> String {
    let sum = Sha256::digest(format!("{}|{}|{}", pr.host, pr.repo, pr.number));
    sum.iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// The last element of a repository name, with the edge cases of Go's
/// `filepath.Base` (existing worktree folders were named with it): trailing
/// separators are ignored, an empty name is "." and a bare separator is kept.
fn base(repo: &str) -> &str {
    if repo.is_empty() {
        return ".";
    }
    let trimmed = repo.trim_end_matches(is_separator);
    if trimmed.is_empty() {
        return &repo[..1];
    }
    trimmed.rsplit(is_separator).next().unwrap_or(trimmed)
}

/// Appends a slash-separated relative name to `root` one element at a time, so
/// it can neither replace the root nor leave mixed separators on Windows.
fn join(root: &Path, name: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(
        name.split(is_separator)
            .filter(|part| !part.is_empty() && *part != "."),
    );
    path
}

/// The deterministic worktree folder of a PR:
/// `<worktree_dir>/<repo>-<number>-<hash>`.
pub fn worktree_path(cfg: &Config, pr: &Item) -> Result<PathBuf> {
    let root = cfg.worktrees()?;
    let name = UNSAFE.replace_all(base(&pr.repo), "_");
    Ok(root.join(format!("{name}-{}-{}", pr.number, hash(pr))))
}

/// The worktree directory of a PR, when it exists on disk.
pub fn worktree_exists(cfg: &Config, pr: &Item) -> Option<PathBuf> {
    worktree_path(cfg, pr).ok().filter(|path| path.is_dir())
}

/// The branch created for a PR worktree.
pub fn local_branch(pr: &Item) -> String {
    format!("pr-tracker/{}-{}", pr.number, hash(pr))
}

fn head_ref(pr: &Item) -> String {
    format!("refs/pr-tracker/heads/{}-{}", pr.number, hash(pr))
}

/// The local clone of the PR's repository and the remote that points at it:
/// the configured path first, then `clone_roots` (`<root>/<name>`,
/// `<root>/<owner>/<name>` and `<root>/<host>/<owner>/<name>`), accepting a
/// candidate only if one of its remotes points to the repository.
pub fn resolve_clone(ctx: &Ctx, cfg: &Config, pr: &Item) -> Option<(PathBuf, String)> {
    if let Some(repo) = cfg
        .repo(&pr.instance, &pr.repo)
        .filter(|repo| !repo.path.is_empty())
    {
        let path = config::expand_home(&repo.path);
        let remote = if repo.remote.is_empty() {
            match_remote(ctx, &path, pr).unwrap_or_else(|| "origin".to_string())
        } else {
            repo.remote.clone()
        };
        return Some((path, remote));
    }
    let name = base(&pr.repo);
    for root in &cfg.clone_roots {
        let root = config::expand_home(root);
        let candidates = [
            join(&root, name),
            join(&root, &pr.repo),
            join(&join(&root, &pr.host), &pr.repo),
        ];
        for candidate in candidates {
            if let Some(remote) = match_remote(ctx, &candidate, pr) {
                return Some((candidate, remote));
            }
        }
    }
    None
}

/// The remote of `dir` whose URL points at the PR repository.
fn match_remote(ctx: &Ctx, dir: &Path, pr: &Item) -> Option<String> {
    if !dir.is_dir() {
        return None;
    }
    let remotes = git(ctx, dir, &["remote", "-v"]).ok()?;
    let want = format!("{}/{}", pr.host, pr.repo).to_lowercase();
    let want_ssh = format!("{}:{}", pr.host, pr.repo).to_lowercase();
    for line in remotes.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(url)) = (fields.next(), fields.next()) else {
            continue;
        };
        // Local remotes on Windows are backslash paths.
        let url = if cfg!(windows) {
            url.replace('\\', "/")
        } else {
            url.to_string()
        };
        let url = url.strip_suffix('/').unwrap_or(&url);
        let url = url.strip_suffix(".git").unwrap_or(url).to_lowercase();
        if url.ends_with(&want) || url.ends_with(&want_ssh) {
            return Some(name.to_string());
        }
    }
    None
}

/// Checks that `path` is a git repository.
pub fn validate_clone(ctx: &Ctx, path: &Path) -> Result<()> {
    let path = path
        .to_str()
        .map_or_else(|| path.to_path_buf(), config::expand_home);
    if !path.is_dir() {
        return Err(msg!("pasta não existe: %s", path.display().to_string()));
    }
    if git(ctx, &path, &["rev-parse", "--git-dir"]).is_err() {
        return Err(msg!(
            "não é um repositório git: %s",
            path.display().to_string()
        ));
    }
    Ok(())
}

/// Fetches the PR head and adds a worktree for it, returning its directory.
/// When the worktree already exists it is fast-forwarded to the latest PR head
/// if clean.
pub fn create_worktree(
    ctx: &Ctx,
    cfg: &Config,
    client: &dyn Client,
    pr: &Item,
    clone: &Path,
    remote: &str,
) -> Result<PathBuf> {
    let worktree = worktree_path(cfg, pr)?;
    // FETCH_HEAD is per-worktree, so the head is fetched into a shared ref.
    let head = head_ref(pr);
    git(
        ctx,
        clone,
        &["fetch", remote, &format!("+{}:{head}", client.head_ref(pr))],
    )?;
    let branch = local_branch(pr);
    if worktree.is_dir() {
        let current = git(
            ctx,
            &worktree,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )?;
        if current != branch {
            return Err(msg!(ERR_OTHER_BRANCH, branch));
        }
        if !git(ctx, &worktree, &["status", "--porcelain"])?.is_empty() {
            return Err(msg!(ERR_DIRTY));
        }
        git(ctx, &worktree, &["merge", "--ff-only", &head])?;
        return Ok(worktree);
    }
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let add: [&OsStr; 6] = [
        "worktree".as_ref(),
        "add".as_ref(),
        "-B".as_ref(),
        branch.as_ref(),
        worktree.as_os_str(),
        head.as_ref(),
    ];
    run(ctx, clone, &add)?;
    // Track the source branch so pull (and push with push.default=upstream)
    // work for same-repo PRs.
    if !pr.from_fork && !pr.source_branch.is_empty() {
        let source = &pr.source_branch;
        let refspec = format!("+{source}:refs/remotes/{remote}/{source}");
        if git(ctx, &worktree, &["fetch", remote, &refspec]).is_ok() {
            let upstream = format!("--set-upstream-to={remote}/{source}");
            let _ = git(ctx, &worktree, &["branch", &upstream]);
        }
    }
    // Make the base branch available for diffs inside the worktree.
    if !pr.target_branch.is_empty() {
        let target = &pr.target_branch;
        let refspec = format!("+{target}:refs/remotes/{remote}/{target}");
        let _ = git(ctx, &worktree, &["fetch", remote, &refspec]);
    }
    Ok(worktree)
}

/// Sends only HEAD to the PR source branch on its configured upstream remote.
/// An explicit refspec avoids push.default and push refspecs sending the
/// generated local branch name or other branches.
pub fn push_worktree(ctx: &Ctx, worktree: &Path, pr: &Item) -> Result<()> {
    let branch = git(
        ctx,
        worktree,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    if branch != local_branch(pr) {
        return Err(msg!(ERR_OTHER_BRANCH, local_branch(pr)));
    }
    let remote = git(
        ctx,
        worktree,
        &["config", "--get", &format!("branch.{branch}.remote")],
    );
    let merge = git(
        ctx,
        worktree,
        &["config", "--get", &format!("branch.{branch}.merge")],
    );
    match (remote, merge) {
        (Ok(remote), Ok(merge))
            if !remote.is_empty()
                && remote != "."
                && !pr.source_branch.is_empty()
                && merge == format!("refs/heads/{}", pr.source_branch) =>
        {
            git(
                ctx,
                worktree,
                &["push", "--", &remote, &format!("HEAD:{merge}")],
            )
            .map(drop)
        }
        _ => Err(msg!(
            "configure o upstream do worktree para a branch de origem %q antes de fazer push",
            &pr.source_branch
        )),
    }
}

/// Removes the PR worktree, its local branch and its fetched head ref.
pub fn remove_worktree(ctx: &Ctx, cfg: &Config, pr: &Item, force: bool) -> Result<()> {
    let Some(worktree) = worktree_exists(cfg, pr) else {
        return Ok(());
    };
    let Ok(common) = git(
        ctx,
        &worktree,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) else {
        // Not a valid worktree anymore: just delete the folder.
        return Ok(std::fs::remove_dir_all(&worktree)?);
    };
    let common = PathBuf::from(common);
    let main = common.parent().unwrap_or(&common);
    if !force
        && git(ctx, &worktree, &["status", "--porcelain"]).is_ok_and(|dirty| !dirty.is_empty())
    {
        return Err(msg!(ERR_DIRTY));
    }
    let mut remove: Vec<&OsStr> = vec!["worktree".as_ref(), "remove".as_ref()];
    if force {
        remove.push("--force".as_ref());
    }
    remove.push(worktree.as_os_str());
    run(ctx, main, &remove)?;
    let _ = git(ctx, main, &["branch", "-D", &local_branch(pr)]);
    let _ = git(ctx, main, &["update-ref", "-d", &head_ref(pr)]);
    let _ = git(ctx, main, &["worktree", "prune"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::MutexGuard;

    use super::*;
    use crate::config::Instance;
    use crate::i18n::tests::env_lock;
    use crate::provider::{MergeOptions, Thread};

    /// Only provides `head_ref`, the single method gitops needs.
    struct FakeClient;

    impl Client for FakeClient {
        fn instance(&self) -> &Instance {
            unimplemented!()
        }
        fn tool(&self) -> &'static str {
            unimplemented!()
        }
        fn list(&self, _: &Ctx) -> Result<Vec<Item>> {
            unimplemented!()
        }
        fn thread(&self, _: &Ctx, _: &Item) -> Result<Thread> {
            unimplemented!()
        }
        fn add_comment(&self, _: &Ctx, _: &Item, _: &str) -> Result<()> {
            unimplemented!()
        }
        fn approve(&self, _: &Ctx, _: &Item) -> Result<()> {
            unimplemented!()
        }
        fn merge(&self, _: &Ctx, _: &Item, _: &MergeOptions) -> Result<()> {
            unimplemented!()
        }
        fn close(&self, _: &Ctx, _: &Item) -> Result<()> {
            unimplemented!()
        }
        fn set_draft(&self, _: &Ctx, _: &Item, _: bool) -> Result<()> {
            unimplemented!()
        }
        fn update_branch(&self, _: &Ctx, _: &Item) -> Result<()> {
            unimplemented!()
        }
        fn checkout(&self, _: &Ctx, _: &Item, _: &Path) -> Result<()> {
            unimplemented!()
        }
        fn head_ref(&self, _: &Item) -> String {
            "refs/pull/5/head".to_string()
        }
        fn auth_status(&self, _: &Ctx) -> Result<()> {
            unimplemented!()
        }
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        git(&Ctx::background(), dir, args).unwrap_or_else(|err| panic!("{err}"))
    }

    fn text(path: &Path) -> &str {
        path.to_str().expect("temporary paths are UTF-8")
    }

    struct Fixture {
        cfg: Config,
        pr: Item,
        clone: PathBuf,
        /// Keeps the repositories on disk for the duration of the test.
        root: tempfile::TempDir,
        _env: MutexGuard<'static, ()>,
    }

    /// An upstream repository with a PR head, and a clone of it under a clone
    /// root. `None` when git is not installed.
    fn setup() -> Option<Fixture> {
        if which::which("git").is_err() {
            eprintln!("skipping: git not installed");
            return None;
        }
        let env = env_lock();
        for (key, value) in [
            ("GIT_AUTHOR_NAME", "t"),
            ("GIT_AUTHOR_EMAIL", "t@t"),
            ("GIT_COMMITTER_NAME", "t"),
            ("GIT_COMMITTER_EMAIL", "t@t"),
        ] {
            // SAFETY: every test that reads or writes the environment holds env_lock.
            unsafe { std::env::set_var(key, value) }
        }
        let root = tempfile::tempdir().unwrap();
        // The upstream path ends in host/owner/repo so remote matching works.
        let up = root
            .path()
            .join("remote")
            .join("github.com")
            .join("o")
            .join("r");
        fs::create_dir_all(&up).unwrap();
        run_git(&up, &["init", "-q", "-b", "main"]);
        fs::write(up.join("a.txt"), "a\n").unwrap();
        run_git(&up, &["add", "."]);
        run_git(&up, &["commit", "-qm", "base"]);
        run_git(&up, &["checkout", "-qb", "feature"]);
        fs::write(up.join("b.txt"), "b\n").unwrap();
        run_git(&up, &["add", "."]);
        run_git(&up, &["commit", "-qm", "feature"]);
        run_git(&up, &["update-ref", "refs/pull/5/head", "HEAD"]);
        run_git(&up, &["checkout", "-q", "main"]);

        let clones = root.path().join("clones");
        let clone = clones.join("r");
        run_git(root.path(), &["clone", "-q", text(&up), text(&clone)]);

        let mut cfg = Config::default();
        cfg.worktree_dir = text(&root.path().join("wt")).to_string();
        cfg.clone_roots = vec![text(&clones).to_string()];
        let pr = Item {
            instance: "gh".into(),
            host: "github.com".into(),
            repo: "o/r".into(),
            number: 5,
            source_branch: "feature".into(),
            target_branch: "main".into(),
            ..Item::default()
        };
        Some(Fixture {
            cfg,
            pr,
            clone,
            root,
            _env: env,
        })
    }

    #[test]
    fn hash_matches_the_go_implementation() {
        // Printed by Go's gitops.Hash for the same host, repository and number.
        let pr = Item {
            host: "github.com".into(),
            repo: "o/r".into(),
            number: 5,
            ..Item::default()
        };
        assert_eq!(hash(&pr), "948cc3d1");
        let pr = Item {
            host: "gitlab.corp.io".into(),
            repo: "grp/sub/proj".into(),
            number: 1234,
            ..Item::default()
        };
        assert_eq!(hash(&pr), "03e37ce8");
    }

    #[test]
    fn base_follows_go_filepath_base() {
        for (repo, want) in [
            ("o/r", "r"),
            ("grp/sub/proj", "proj"),
            ("r", "r"),
            ("o/r/", "r"),
            ("", "."),
            ("/", "/"),
        ] {
            assert_eq!(base(repo), want, "base({repo:?})");
        }
    }

    #[test]
    fn worktree_lifecycle() {
        let Some(Fixture {
            cfg,
            pr,
            clone,
            root: _root,
            _env,
        }) = setup()
        else {
            return;
        };
        let ctx = Ctx::background();

        let (path, remote) = resolve_clone(&ctx, &cfg, &pr).expect("clone not resolved");
        assert_eq!(
            (path.as_path(), remote.as_str()),
            (clone.as_path(), "origin")
        );
        let worktree = create_worktree(&ctx, &cfg, &FakeClient, &pr, &path, &remote).unwrap();
        assert_eq!(worktree, worktree_path(&cfg, &pr).unwrap());
        assert_eq!(
            worktree.file_name().unwrap().to_str().unwrap(),
            format!("r-5-{}", hash(&pr))
        );
        assert!(worktree.join("b.txt").exists(), "PR head not checked out");
        assert_eq!(
            run_git(&worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/feature"
        );
        // Running again reuses the existing worktree.
        let again = create_worktree(&ctx, &cfg, &FakeClient, &pr, &path, &remote).unwrap();
        assert_eq!(again, worktree);

        fs::write(worktree.join("dirty.txt"), "x").unwrap();
        let err = remove_worktree(&ctx, &cfg, &pr, false).unwrap_err();
        assert!(err.is(ERR_DIRTY), "expected ERR_DIRTY, got {err}");
        remove_worktree(&ctx, &cfg, &pr, true).unwrap();
        assert_eq!(worktree_exists(&cfg, &pr), None, "worktree still exists");
        assert_eq!(
            run_git(&clone, &["branch", "--list", &local_branch(&pr)]),
            ""
        );
    }

    #[test]
    fn resolve_clone_configured_path() {
        let Some(Fixture {
            mut cfg,
            pr,
            clone,
            root: _root,
            _env,
        }) = setup()
        else {
            return;
        };
        let ctx = Ctx::background();
        cfg.clone_roots.clear();
        assert_eq!(
            resolve_clone(&ctx, &cfg, &pr),
            None,
            "expected no clone without mapping"
        );
        cfg.set_repo_path("gh", "o/r", text(&clone));
        let (path, _) = resolve_clone(&ctx, &cfg, &pr).expect("mapped clone not resolved");
        assert_eq!(path, clone);
    }

    #[test]
    fn worktree_update_from_source() {
        for scenario in [
            "new commits",
            "unchanged",
            "dirty",
            "diverged",
            "different branch",
            "fetch failure",
        ] {
            let Some(Fixture {
                cfg,
                mut pr,
                clone,
                root: _root,
                _env,
            }) = setup()
            else {
                return;
            };
            let ctx = Ctx::background();
            let worktree = create_worktree(&ctx, &cfg, &FakeClient, &pr, &clone, "origin").unwrap();
            let mut before = run_git(&worktree, &["rev-parse", "HEAD"]);
            let up = PathBuf::from(run_git(&clone, &["remote", "get-url", "origin"]));
            if scenario != "unchanged" {
                run_git(&up, &["checkout", "-q", "feature"]);
                fs::write(up.join("remote.txt"), "new commit\n").unwrap();
                run_git(&up, &["add", "remote.txt"]);
                run_git(&up, &["commit", "-qm", "remote update"]);
                run_git(&up, &["update-ref", "refs/pull/5/head", "HEAD"]);
            }
            if scenario == "dirty" || scenario == "diverged" {
                fs::write(worktree.join("local.txt"), "local work\n").unwrap();
                if scenario == "diverged" {
                    run_git(&worktree, &["add", "local.txt"]);
                    run_git(&worktree, &["commit", "-qm", "local commit"]);
                    before = run_git(&worktree, &["rev-parse", "HEAD"]);
                }
            }
            if scenario == "different branch" {
                run_git(&worktree, &["checkout", "-qb", "other"]);
            }
            let missing = tempfile::tempdir().unwrap();
            if scenario == "fetch failure" {
                let gone = missing.path().join("missing");
                run_git(&clone, &["remote", "set-url", "origin", text(&gone)]);
            }
            // Forks also use the fetched PR ref, not a same-named branch on origin.
            pr.from_fork = true;
            let updated = create_worktree(&ctx, &cfg, &FakeClient, &pr, &clone, "origin");
            let after = run_git(&worktree, &["rev-parse", "HEAD"]);
            if scenario == "new commits" || scenario == "unchanged" {
                let want = run_git(&up, &["rev-parse", "refs/pull/5/head"]);
                assert_eq!(updated.as_ref(), Ok(&worktree), "{scenario}");
                assert_eq!(after, want, "{scenario}");
                continue;
            }
            let err = updated.expect_err(scenario);
            assert_eq!(after, before, "{scenario}: head moved by a failed update");
            if scenario == "dirty" {
                assert!(err.is(ERR_DIRTY), "expected ERR_DIRTY, got {err}");
            }
            if scenario == "dirty" || scenario == "diverged" {
                let contents = fs::read_to_string(worktree.join("local.txt")).unwrap();
                assert_eq!(contents, "local work\n", "{scenario}: local work lost");
            }
        }
    }

    #[test]
    fn push_worktree_uses_source_branch_without_force() {
        for scenario in [
            "push",
            "fork upstream",
            "missing upstream",
            "wrong upstream",
            "diverged",
        ] {
            let Some(Fixture {
                cfg,
                mut pr,
                clone,
                root: _root,
                _env,
            }) = setup()
            else {
                return;
            };
            let ctx = Ctx::background();
            let worktree = create_worktree(&ctx, &cfg, &FakeClient, &pr, &clone, "origin").unwrap();
            let mut up = PathBuf::from(run_git(&clone, &["remote", "get-url", "origin"]));
            let mut before = run_git(&up, &["rev-parse", "feature"]);
            let main = run_git(&up, &["rev-parse", "main"]);
            fs::write(worktree.join("local.txt"), "local commit\n").unwrap();
            run_git(&worktree, &["add", "local.txt"]);
            run_git(&worktree, &["commit", "-qm", "local update"]);
            let local = run_git(&worktree, &["rev-parse", "HEAD"]);
            let forks = tempfile::tempdir().unwrap();
            match scenario {
                "fork upstream" => {
                    pr.from_fork = true;
                    let fork = forks.path().join("fork.git");
                    run_git(&clone, &["clone", "--bare", "-q", text(&up), text(&fork)]);
                    run_git(&worktree, &["remote", "add", "fork", text(&fork)]);
                    run_git(&worktree, &["fetch", "-q", "fork"]);
                    run_git(&worktree, &["branch", "--set-upstream-to=fork/feature"]);
                    up = fork;
                }
                "missing upstream" => {
                    run_git(&worktree, &["branch", "--unset-upstream"]);
                }
                "wrong upstream" => {
                    run_git(&worktree, &["branch", "--set-upstream-to=origin/main"]);
                }
                "diverged" => {
                    run_git(&up, &["checkout", "-q", "feature"]);
                    run_git(&up, &["commit", "--allow-empty", "-qm", "remote update"]);
                    run_git(&up, &["checkout", "-q", "main"]);
                    before = run_git(&up, &["rev-parse", "feature"]);
                }
                _ => {}
            }
            // Explicit destination must override even a conflicting push refspec.
            run_git(
                &worktree,
                &["config", "remote.origin.push", "HEAD:refs/heads/main"],
            );
            let pushed = push_worktree(&ctx, &worktree, &pr);
            let want = if scenario == "push" || scenario == "fork upstream" {
                pushed.unwrap_or_else(|err| panic!("{scenario}: {err}"));
                &local
            } else {
                assert!(pushed.is_err(), "{scenario}: unsafe push accepted");
                &before
            };
            assert_eq!(
                &run_git(&up, &["rev-parse", "feature"]),
                want,
                "{scenario}: source branch"
            );
            assert_eq!(
                run_git(&up, &["rev-parse", "main"]),
                main,
                "{scenario}: base changed"
            );
            assert_eq!(
                run_git(&worktree, &["rev-parse", "HEAD"]),
                local,
                "{scenario}: lost"
            );
            assert_eq!(
                run_git(&up, &["branch", "--list", &local_branch(&pr)]),
                "",
                "{scenario}: generated local branch was pushed"
            );
        }
    }
}
