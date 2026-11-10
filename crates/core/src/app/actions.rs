use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::Application;
use crate::error::{Error, Result};
use crate::gitops;
use crate::msg;
use crate::provider::{self, Client, Item, MergeOptions, Thread};
use crate::toolchain::{self, Ctx};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Comment,
    Approve,
    Merge,
    Close,
    MarkReady,
    MarkDraft,
    Checkout,
    UpdateWorktree,
    UpdateBranch,
    PrepareTerminal,
    PrepareDiff,
    RemoveWorktree,
}

/// An already-confirmed operation. `force` is accepted only for a standalone
/// removal, after a separate confirmation of local data loss.
#[derive(Debug, Clone)]
pub struct Command {
    pub action: Action,
    pub item: Item,
    pub body: String,
    pub remove_after: bool,
    pub force: bool,
    pub push: bool,
}

impl Command {
    pub fn new(action: Action) -> Self {
        Command {
            action,
            item: Item::default(),
            body: String::new(),
            remove_after: false,
            force: false,
            push: false,
        }
    }
}

/// What an operation did. `error` can accompany a message: a remote action may
/// succeed even when its subsequent local cleanup fails, and callers must
/// honor `refresh` in either case.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub message: String,
    pub refresh: bool,
    pub reload_thread: bool,
    pub directory: Option<PathBuf>,
    pub args: Vec<String>,
    pub note: String,
    pub error: Option<Error>,
}

impl Outcome {
    fn failed(error: Error) -> Self {
        Outcome {
            error: Some(error),
            ..Outcome::default()
        }
    }

    fn fail(mut self, error: Error) -> Self {
        self.error = Some(error);
        self
    }
}

/// Catalog key of the error for a repository without a known local clone.
pub const ERR_CLONE_REQUIRED: &str = "pasta local do repositório não configurada";
/// Catalog key of the error for a worktree with uncommitted changes.
pub const ERR_DIRTY_WORKTREE: &str = gitops::ERR_DIRTY;

impl Application {
    fn item_client(&self, item: &Item) -> Result<Arc<dyn Client>> {
        let Some(instance) = self
            .cfg
            .instance(&item.instance)
            .filter(|instance| !instance.disabled)
        else {
            return Err(msg!("instância indisponível: %s", &item.instance));
        };
        if item.provider != instance.provider || item.host != instance.host {
            return Err(msg!(
                "a instância mudou; atualize o item antes de continuar"
            ));
        }
        Ok(self.client(instance))
    }

    /// Loads the conversation without presentation-specific Markdown cleanup.
    pub fn detail(&self, ctx: &Ctx, item: &Item) -> Result<Thread> {
        let ctx = ctx.with_paths(&self.cfg.tool_paths);
        ctx.check()?;
        self.item_client(item)?.thread(&ctx, item)
    }

    pub fn merge_options(&self, item: &Item) -> MergeOptions {
        let mut options = MergeOptions {
            method: "merge".into(),
            ..MergeOptions::default()
        };
        if let Some(instance) = self.cfg.instance(&item.instance) {
            if !instance.merge_method.is_empty() {
                options.method = instance.merge_method.clone();
            }
            options.auto = instance.auto_merge;
            options.delete_branch = instance.delete_branch;
        }
        if let Some(repo) = self
            .cfg
            .repo(&item.instance, &item.repo)
            .filter(|repo| !repo.merge_method.is_empty())
        {
            options.method = repo.merge_method.clone();
        }
        options
    }

    pub fn execute(&self, ctx: &Ctx, command: &Command) -> Outcome {
        let ctx = &ctx.with_paths(&self.cfg.tool_paths);
        if let Some(err) = ctx.err() {
            return Outcome::failed(err);
        }
        let action = command.action;
        if command.force && action != Action::RemoveWorktree {
            return Outcome::failed(msg!("remoção forçada exige um comando separado"));
        }
        if command.push && action != Action::UpdateWorktree {
            return Outcome::failed(msg!("push disponível apenas ao atualizar a branch"));
        }
        if command.remove_after
            && !matches!(action, Action::Approve | Action::Merge | Action::Close)
        {
            return Outcome::failed(msg!(
                "limpeza combinada exige aprovação, merge ou fechamento"
            ));
        }
        let item = &command.item;
        if item.is_issue() && action != Action::Comment {
            return Outcome::failed(msg!("ação disponível apenas para pull/merge requests"));
        }
        if item.repo.is_empty() || item.number <= 0 {
            return Outcome::failed(msg!("item inválido"));
        }
        let client = match self.item_client(item) {
            Ok(client) => client,
            Err(err) => return Outcome::failed(err),
        };
        let reference = item.reference();
        let mut out = Outcome::default();
        let remote = match action {
            Action::Comment => {
                let body = command.body.trim();
                if body.is_empty() {
                    return Outcome::failed(msg!("comentário vazio"));
                }
                return match client.add_comment(ctx, item, body) {
                    Ok(()) => Outcome {
                        message: self.t("comentário enviado em ") + &item.repo + &reference,
                        refresh: true,
                        reload_thread: true,
                        ..out
                    },
                    Err(err) => Outcome::failed(err),
                };
            }
            Action::Approve => {
                out.message = self.t("aprovado ") + &reference;
                client.approve(ctx, item)
            }
            Action::Merge => {
                out.message = self.t("merge de ") + &reference + &self.t(" feito");
                client.merge(ctx, item, &self.merge_options(item))
            }
            Action::Close => {
                out.message = reference.clone() + &self.t(" fechado sem merge");
                client.close(ctx, item)
            }
            Action::MarkReady | Action::MarkDraft => {
                let draft = action == Action::MarkDraft;
                out.message = reference.clone()
                    + &self.t(if draft {
                        " marcado como Draft"
                    } else {
                        " marcado como Ready"
                    });
                client.set_draft(ctx, item, draft)
            }
            Action::UpdateBranch => {
                out.message = self.t("atualização com a branch base solicitada em ")
                    + &item.repo
                    + &reference;
                client.update_branch(ctx, item)
            }
            Action::Checkout => {
                let Some((clone, _)) = (self.hooks.resolve_clone)(ctx, &self.cfg, item) else {
                    return Outcome::failed(msg!(ERR_CLONE_REQUIRED));
                };
                return match client.checkout(ctx, item, &clone) {
                    Ok(()) => Outcome {
                        message: format!(
                            "branch {}{}{}",
                            item.source_branch,
                            self.t(" em "),
                            clone.display()
                        ),
                        ..out
                    },
                    Err(err) => Outcome::failed(err),
                };
            }
            Action::UpdateWorktree | Action::PrepareTerminal | Action::PrepareDiff => {
                let update = action == Action::UpdateWorktree;
                let dir = match self.ensure_worktree(ctx, client.as_ref(), item, update) {
                    Ok(dir) => dir,
                    Err(err) => return Outcome::failed(err),
                };
                out.directory = Some(dir.clone());
                if update {
                    out.message = self.t("worktree pronto: ") + &dir.display().to_string();
                    if command.push {
                        if let Err(err) = (self.hooks.push_worktree)(ctx, &dir, item) {
                            return out
                                .fail(msg!("branch local atualizada, mas o push falhou: %w", err));
                        }
                        out.message += &self.t(" · push concluído");
                        out.refresh = true;
                    }
                }
                if action == Action::PrepareDiff {
                    (out.args, out.note) = self.diff_command(ctx, &dir, item);
                }
                return out;
            }
            Action::RemoveWorktree => {
                return match (self.hooks.remove_worktree)(ctx, &self.cfg, item, command.force) {
                    Ok(()) => {
                        out.message = self.t("worktree removido");
                        if command.force {
                            out.message += &self.t(" (alterações descartadas)");
                        }
                        out
                    }
                    Err(err) => Outcome::failed(err),
                };
            }
        };
        if let Err(err) = remote {
            return Outcome::failed(err);
        }
        out.refresh = true;
        if command.remove_after {
            if let Err(err) = (self.hooks.remove_worktree)(ctx, &self.cfg, item, false) {
                return out.fail(err);
            }
            out.message += &self.t(" · worktree removido");
        }
        out
    }

    fn ensure_worktree(
        &self,
        ctx: &Ctx,
        client: &dyn Client,
        item: &Item,
        update: bool,
    ) -> Result<PathBuf> {
        if !update && let Some(dir) = (self.hooks.worktree_exists)(&self.cfg, item) {
            return Ok(dir);
        }
        let Some((clone, remote)) = (self.hooks.resolve_clone)(ctx, &self.cfg, item) else {
            return Err(msg!(ERR_CLONE_REQUIRED));
        };
        (self.hooks.create_worktree)(ctx, &self.cfg, client, item, &clone, &remote)
    }

    /// The command that shows the PR's changes against its merge base, and a
    /// note when the preferred diff tool is unavailable.
    fn diff_command(&self, ctx: &Ctx, worktree: &Path, pr: &Item) -> (Vec<String>, String) {
        let mut base = "HEAD~1".to_string();
        let upstream = gitops::git(ctx, worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]);
        if upstream.is_ok() || !pr.target_branch.is_empty() {
            let remote = upstream
                .as_deref()
                .ok()
                .and_then(|upstream| upstream.split_once('/'))
                .map(|(remote, _)| remote)
                .filter(|remote| !remote.is_empty())
                .unwrap_or("origin");
            let target = format!("{remote}/{}", pr.target_branch);
            if let Ok(merge_base) = gitops::git(ctx, worktree, &["merge-base", &target, "HEAD"]) {
                base = merge_base;
            }
        }
        let git_diff = vec!["git".to_string(), "diff".to_string(), base.clone()];
        if self.cfg.diff_tool != "hunk" {
            return (git_diff, String::new());
        }
        match toolchain::lookup(ctx, "hunk") {
            Ok(path) => (
                vec![path.to_string_lossy().into_owned(), "diff".into(), base],
                String::new(),
            ),
            Err(_) => {
                let hint = self.t(provider::install_hint("hunk"));
                (
                    git_diff,
                    self.t("hunk não instalado (") + &hint + &self.t("); usando git diff"),
                )
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::tests::FakeClient;
    use super::super::{Application, Hooks};
    use super::*;
    use crate::config::{Config, Instance, Provider, Repo};
    use crate::provider::Kind;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    pub(crate) fn fixture() -> (Config, Hooks, Arc<FakeClient>, Item) {
        let mut cfg = Config::default();
        cfg.language = "pt".into();
        cfg.instances = vec![Instance {
            name: "work".into(),
            provider: Provider::GitHub,
            host: "github.com".into(),
            merge_method: "squash".into(),
            auto_merge: true,
            delete_branch: true,
            ..Instance::default()
        }];
        cfg.repos = vec![Repo {
            instance: "work".into(),
            name: "org/repo".into(),
            merge_method: "rebase".into(),
            ..Repo::default()
        }];
        let client = Arc::new(FakeClient::default());
        let shared = client.clone();
        let hooks = Hooks {
            client: Arc::new(move |_, _| shared.clone()),
            resolve_clone: Arc::new(|_, _, _| Some((PathBuf::from("/clone"), "upstream".into()))),
            worktree_exists: Arc::new(|_, _| Some(PathBuf::from("/worktree"))),
            create_worktree: Arc::new(|_, _, _, _, _, _| Ok(PathBuf::from("/created"))),
            push_worktree: Arc::new(|_, _, _| Ok(())),
            remove_worktree: Arc::new(|_, _, _, _| Ok(())),
        };
        let item = Item {
            instance: "work".into(),
            provider: Provider::GitHub,
            host: "github.com".into(),
            repo: "org/repo".into(),
            number: 7,
            ..Item::default()
        };
        (cfg, hooks, client, item)
    }

    fn command(action: Action, item: &Item) -> Command {
        Command {
            item: item.clone(),
            ..Command::new(action)
        }
    }

    fn name(action: Action) -> &'static str {
        match action {
            Action::Approve => "approve",
            Action::Merge => "merge",
            Action::Close => "close",
            Action::MarkReady => "mark_ready",
            Action::MarkDraft => "mark_draft",
            Action::UpdateBranch => "update_branch",
            _ => unreachable!(),
        }
    }

    #[test]
    fn remote_action_and_cleanup_outcomes() {
        for action in [Action::Approve, Action::Merge, Action::Close] {
            for scenario in ["success", "remote failure", "dirty", "cleanup failure"] {
                let (cfg, mut hooks, client, item) = fixture();
                let failure = Error::external("failure");
                if scenario == "remote failure" {
                    *client.fail.lock().unwrap() = Some(failure.clone());
                }
                let cleanup = Arc::new(AtomicUsize::new(0));
                let (count, calls, cleanup_failure) =
                    (cleanup.clone(), client.clone(), failure.clone());
                hooks.remove_worktree = Arc::new(move |_, _, _, force| {
                    count.fetch_add(1, Ordering::SeqCst);
                    assert!(!force, "combined operation forced removal");
                    assert_eq!(calls.calls().len(), 1, "cleanup ran before remote action");
                    match scenario {
                        "dirty" => Err(msg!(ERR_DIRTY_WORKTREE)),
                        "cleanup failure" => Err(cleanup_failure.clone()),
                        _ => Ok(()),
                    }
                });
                let app = Application::new(&cfg, None).with_hooks(hooks);
                let out = app.execute(
                    &Ctx::background(),
                    &Command {
                        remove_after: true,
                        ..command(action, &item)
                    },
                );
                assert_eq!(client.calls(), [name(action)], "{action:?}/{scenario}");
                if action == Action::Merge {
                    let expected = MergeOptions {
                        method: "rebase".into(),
                        auto: true,
                        delete_branch: true,
                    };
                    assert_eq!(*client.options.lock().unwrap(), expected);
                }
                let cleanups = cleanup.load(Ordering::SeqCst);
                if scenario == "remote failure" {
                    assert_eq!(
                        (cleanups, out.refresh, out.message.as_str(), out.error),
                        (0, false, "", Some(failure))
                    );
                    continue;
                }
                assert!(
                    cleanups == 1 && out.refresh && !out.message.is_empty(),
                    "lost remote success: {out:?}"
                );
                match scenario {
                    "success" => assert_eq!(out.error, None),
                    "dirty" => assert!(out.error.is_some_and(|err| err.is(ERR_DIRTY_WORKTREE))),
                    _ => assert_eq!(out.error, Some(failure)),
                }
            }
        }
    }

    #[test]
    fn execute_rejects_invalid_commands_before_side_effects() {
        for scenario in [
            "issue approval",
            "empty comment",
            "force merge",
            "combined comment",
            "missing instance",
            "disabled instance",
            "changed host",
            "changed provider",
            "invalid item",
            "cancelled",
        ] {
            let (mut cfg, hooks, client, item) = fixture();
            let mut command = command(Action::Approve, &item);
            let cancel = Arc::new(AtomicBool::new(false));
            match scenario {
                "issue approval" => command.item.kind = Kind::Issue,
                "empty comment" => {
                    command.action = Action::Comment;
                    command.body = " \n ".into();
                }
                "force merge" => {
                    command.action = Action::Merge;
                    command.force = true;
                }
                "combined comment" => {
                    command.action = Action::Comment;
                    command.body = "hello".into();
                    command.remove_after = true;
                }
                "missing instance" => command.item.instance = "missing".into(),
                "disabled instance" => cfg.instances[0].disabled = true,
                "changed host" => command.item.host = "another.example".into(),
                "changed provider" => command.item.provider = Provider::GitLab,
                "invalid item" => command.item.number = 0,
                _ => cancel.store(true, Ordering::SeqCst),
            }
            let app = Application::new(&cfg, None).with_hooks(hooks);
            let out = app.execute(&Ctx::background().with_cancel(cancel), &command);
            assert!(out.error.is_some(), "expected rejection: {scenario}");
            assert!(client.calls().is_empty(), "provider called: {scenario}");
        }
    }

    #[test]
    fn comment_and_detail_for_issue() {
        let (cfg, hooks, client, mut item) = fixture();
        item.kind = Kind::Issue;
        let app = Application::new(&cfg, None).with_hooks(hooks);
        let out = app.execute(
            &Ctx::background(),
            &Command {
                body: "  **hello**\n ".into(),
                ..command(Action::Comment, &item)
            },
        );
        assert!(out.error.is_none() && out.refresh && out.reload_thread);
        assert_eq!(*client.body.lock().unwrap(), "**hello**");
        assert_eq!(
            app.detail(&Ctx::background(), &item).unwrap().body,
            "<details>raw markdown</details>"
        );
    }

    #[test]
    fn readiness_and_branch_actions_refresh_only_after_success() {
        for action in [Action::MarkReady, Action::MarkDraft, Action::UpdateBranch] {
            for fail in [false, true] {
                let (cfg, hooks, client, item) = fixture();
                let failure = Error::external("not permitted");
                if fail {
                    *client.fail.lock().unwrap() = Some(failure.clone());
                }
                let out = Application::new(&cfg, None)
                    .with_hooks(hooks)
                    .execute(&Ctx::background(), &command(action, &item));
                assert_eq!(client.calls(), [name(action)]);
                if fail {
                    assert_eq!(
                        (out.error, out.refresh, out.message.as_str()),
                        (Some(failure), false, "")
                    );
                } else {
                    assert!(
                        out.error.is_none()
                            && out.refresh
                            && !out.reload_thread
                            && !out.message.is_empty()
                    );
                }
            }
            let (cfg, hooks, client, item) = fixture();
            let app = Application::new(&cfg, None).with_hooks(hooks);
            let issue = Item {
                kind: Kind::Issue,
                ..item.clone()
            };
            for invalid in [
                command(action, &issue),
                Command {
                    remove_after: true,
                    ..command(action, &item)
                },
                Command {
                    force: true,
                    ..command(action, &item)
                },
            ] {
                assert!(app.execute(&Ctx::background(), &invalid).error.is_some());
                assert!(client.calls().is_empty());
            }
        }
    }

    #[test]
    fn worktree_preparation_and_checkout() {
        for action in [
            Action::UpdateWorktree,
            Action::PrepareTerminal,
            Action::Checkout,
        ] {
            let (cfg, mut hooks, client, item) = fixture();
            let created = Arc::new(AtomicBool::new(false));
            let flag = created.clone();
            hooks.create_worktree = Arc::new(move |_, _, _, _, clone, remote| {
                flag.store(true, Ordering::SeqCst);
                assert_eq!((clone, remote), (Path::new("/clone"), "upstream"));
                Ok(PathBuf::from("/created"))
            });
            let out = Application::new(&cfg, None)
                .with_hooks(hooks)
                .execute(&Ctx::background(), &command(action, &item));
            assert_eq!(out.error, None);
            let created = created.load(Ordering::SeqCst);
            match action {
                Action::UpdateWorktree => {
                    assert!(created && out.directory == Some(PathBuf::from("/created")))
                }
                Action::PrepareTerminal => assert!(
                    !created && out.directory == Some(PathBuf::from("/worktree")),
                    "existing worktree not reused"
                ),
                _ => assert_eq!(client.calls(), ["checkout:/clone"]),
            }
        }
    }

    #[test]
    fn update_worktree_with_optional_push() {
        for scenario in ["local only", "push", "update failure", "push failure"] {
            let (cfg, mut hooks, _, item) = fixture();
            let failure = Error::external("failure");
            let (updated, pushed) = (
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
            );
            let (flag, update_failure) = (updated.clone(), failure.clone());
            hooks.create_worktree = Arc::new(move |_, _, _, _, _, _| {
                flag.store(true, Ordering::SeqCst);
                if scenario == "update failure" {
                    Err(update_failure.clone())
                } else {
                    Ok(PathBuf::from("/created"))
                }
            });
            let (was_updated, flag, push_failure, key) =
                (updated.clone(), pushed.clone(), failure.clone(), item.key());
            hooks.push_worktree = Arc::new(move |_, dir, pr| {
                assert!(
                    was_updated.load(Ordering::SeqCst)
                        && dir == Path::new("/created")
                        && pr.key() == key
                );
                flag.store(true, Ordering::SeqCst);
                if scenario == "push failure" {
                    Err(push_failure.clone())
                } else {
                    Ok(())
                }
            });
            let push = scenario != "local only";
            let out = Application::new(&cfg, None).with_hooks(hooks).execute(
                &Ctx::background(),
                &Command {
                    push,
                    ..command(Action::UpdateWorktree, &item)
                },
            );
            let pushed = pushed.load(Ordering::SeqCst);
            assert!(updated.load(Ordering::SeqCst));
            assert_eq!(pushed, scenario == "push" || scenario == "push failure");
            match scenario {
                "update failure" => assert_eq!(
                    (out.error, out.message.as_str(), out.refresh),
                    (Some(failure), "", false)
                ),
                "push failure" => {
                    assert!(out.error.is_some_and(|err| err.contains(&failure)));
                    assert!(
                        out.directory == Some(PathBuf::from("/created"))
                            && !out.message.is_empty()
                            && !out.refresh
                    );
                }
                _ => assert!(
                    out.error.is_none()
                        && out.refresh == pushed
                        && !out.reload_thread
                        && !out.message.is_empty()
                ),
            }
        }
        for action in [
            Action::Comment,
            Action::Checkout,
            Action::PrepareTerminal,
            Action::RemoveWorktree,
        ] {
            let (cfg, hooks, client, item) = fixture();
            let out = Application::new(&cfg, None).with_hooks(hooks).execute(
                &Ctx::background(),
                &Command {
                    push: true,
                    ..command(action, &item)
                },
            );
            assert!(
                out.error.is_some() && client.calls().is_empty(),
                "push accepted for {action:?}"
            );
        }
    }

    #[test]
    fn missing_clone_and_forced_removal() {
        let (cfg, mut hooks, _, item) = fixture();
        hooks.worktree_exists = Arc::new(|_, _| None);
        hooks.resolve_clone = Arc::new(|_, _, _| None);
        let forced = Arc::new(Mutex::new(Vec::new()));
        let seen = forced.clone();
        hooks.remove_worktree = Arc::new(move |_, _, _, force| {
            seen.lock().unwrap().push(force);
            if force {
                Ok(())
            } else {
                Err(msg!(ERR_DIRTY_WORKTREE))
            }
        });
        let app = Application::new(&cfg, None).with_hooks(hooks);
        for action in [
            Action::Checkout,
            Action::UpdateWorktree,
            Action::PrepareTerminal,
            Action::PrepareDiff,
        ] {
            let out = app.execute(&Ctx::background(), &command(action, &item));
            assert!(
                out.error.is_some_and(|err| err.is(ERR_CLONE_REQUIRED)),
                "{action:?}"
            );
        }
        let out = app.execute(&Ctx::background(), &command(Action::RemoveWorktree, &item));
        assert!(out.error.is_some_and(|err| err.is(ERR_DIRTY_WORKTREE)));
        let out = app.execute(
            &Ctx::background(),
            &Command {
                force: true,
                ..command(Action::RemoveWorktree, &item)
            },
        );
        assert!(out.error.is_none() && !out.refresh && !out.message.is_empty());
        assert_eq!(*forced.lock().unwrap(), [false, true]);
    }
}
