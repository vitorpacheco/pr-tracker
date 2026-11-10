//! The subcommands that print to the terminal instead of opening the window.

use std::time::Duration;

use pr_tracker_core::config::{self, Config, Instance, Provider};
use pr_tracker_core::provider::{self, Relation};
use pr_tracker_core::toolchain::{self, Ctx};
use pr_tracker_core::{cache, gitops};

pub const VERSION: &str = match option_env!("PR_TRACKER_VERSION") {
    Some(version) => version,
    None => "dev",
};

pub const USAGE: &str = "pr-tracker — acompanhe pull/merge requests e issues (GitHub, GitLab, Gitea)

Uso:
  pr-tracker                          abre a interface
  pr-tracker list                     lista PRs e issues no stdout
  pr-tracker doctor                   verifica CLIs, autenticação e configuração
  pr-tracker config path              mostra o caminho do arquivo de configuração
  pr-tracker instance list
  pr-tracker instance add --provider github|gitlab|gitea [--host H] [--name N] [--merge-method M]
  pr-tracker instance rm NOME
  pr-tracker repo set --instance NOME --repo owner/repo --path ~/code/repo [--remote origin]
  pr-tracker repo list
  pr-tracker version
";

/// Reports whether `command` is handled here rather than by the window.
pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "-h" | "--help" | "help" | "version" | "--version" | "config" | "doctor" | "list" | "ls" | "instance" | "instances" | "repo" | "repos"
    )
}

/// Loads the configuration, seeding and saving it on first use.
pub fn load_config() -> Result<Config, String> {
    let (mut cfg, created) = Config::load().map_err(|err| err.to_string())?;
    if created {
        seed(&mut cfg);
        cfg.save().map_err(|err| err.to_string())?;
    }
    Ok(cfg)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let cfg = load_config()?;
    match args[0].as_str() {
        "-h" | "--help" | "help" => print!("{USAGE}"),
        "version" | "--version" => println!("{VERSION}"),
        "config" => println!("{}", cfg.file_path().display()),
        "doctor" => doctor(&cfg),
        "list" | "ls" => list(&cfg),
        "instance" | "instances" => return instance(cfg, &args[1..]),
        "repo" | "repos" => return repo(cfg, &args[1..]),
        other => {
            print!("{USAGE}");
            return Err(format!("comando desconhecido {other:?}"));
        }
    }
    Ok(())
}

/// Creates instances for the hosts the installed CLIs are logged in to.
pub fn seed(cfg: &mut Config) {
    let ctx = Ctx::background().with_paths(&cfg.tool_paths).with_timeout(Duration::from_secs(15));
    for provider in [Provider::GitHub, Provider::GitLab] {
        let host = provider.default_host().to_string();
        let instance = Instance { name: host.clone(), provider, host, ..Instance::default() };
        let client = provider::new(&instance, &[]);
        if toolchain::lookup(&ctx, client.tool()).is_ok() && client.auth_status(&ctx).is_ok() {
            let _ = cfg.upsert_instance("", instance);
        }
    }
    // Gitea has no SaaS host worth probing blindly, so the seed comes from the
    // servers tea is already logged in to.
    for instance in provider::gitea_instances(&ctx) {
        if provider::new(&instance, &[]).auth_status(&ctx).is_ok() {
            let _ = cfg.upsert_instance("", instance);
        }
    }
}

fn mark(ok: bool) -> &'static str {
    if ok { "✔" } else { "✘" }
}

fn or_dash(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}

/// Aligns rows in columns two spaces apart.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0; columns];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            out.push_str(cell);
            if index + 1 < row.len() {
                out.extend(std::iter::repeat_n(' ', widths[index] - cell.chars().count() + 2));
            }
        }
        out.push('\n');
    }
    out
}

fn doctor(cfg: &Config) {
    println!("configuração: {}", cfg.file_path().display());
    let cache = cache::path().map_or_else(|err| format!("indisponível: {err}"), |path| path.display().to_string());
    println!("cache:        {cache}");
    println!("worktrees:    {}", cfg.worktrees().map(|dir| dir.display().to_string()).unwrap_or_default());
    println!();
    let ctx = Ctx::background().with_paths(&cfg.tool_paths);
    for tool in ["git", "gh", "glab", "tea", "hunk", "herdr", "tmux"] {
        let found = toolchain::lookup(&ctx, tool).is_ok();
        let mut line = format!("{} {tool:<6}", mark(found));
        if !found && tool != "git" {
            line += &format!("  {}", provider::install_hint(tool));
        }
        println!("{line}");
    }
    println!();
    let ctx = ctx.with_timeout(Duration::from_secs(30));
    for instance in &cfg.instances {
        let status = provider::new(instance, &[]).auth_status(&ctx);
        let mut message = status.as_ref().map_or_else(|err| err.to_string(), |_| "ok".to_string());
        if instance.disabled {
            message += " (desativada)";
        }
        println!("{} {} [{} {}] {message}", mark(status.is_ok()), instance.name, instance.provider, instance.host);
    }
    if cfg.instances.is_empty() {
        println!("nenhuma instância configurada: pr-tracker instance add --provider github");
    }
}

fn list(cfg: &Config) {
    let ctx = Ctx::background().with_paths(&cfg.tool_paths).with_timeout(Duration::from_secs(120));
    let header = ["INSTÂNCIA", "TIPO", "REPO", "Nº", "CI", "REVISÃO", "COMENT.", "RELAÇÃO", "TÍTULO"];
    let mut rows = vec![header.map(String::from).to_vec()];
    let mut errors = Vec::new();
    for instance in cfg.instances.iter().filter(|instance| !instance.disabled) {
        let items = match provider::new(instance, &cfg.repos).list(&ctx) {
            Ok(items) => items,
            Err(err) => {
                errors.push(format!("{}: {err}", instance.name));
                continue;
            }
        };
        for item in items {
            let relations: Vec<&str> = [
                (Relation::REVIEW_REQUESTED, "revisor"),
                (Relation::AUTHORED, "autor"),
                (Relation::ASSIGNED, "atribuído"),
                (Relation::MENTIONED, "mencionado"),
            ]
            .into_iter()
            .filter(|(relation, _)| item.relations.contains(*relation))
            .map(|(_, label)| label)
            .collect();
            rows.push(vec![
                item.instance.clone(),
                if item.is_issue() { "issue" } else { "pr" }.to_string(),
                item.repo.clone(),
                item.reference(),
                or_dash(item.ci.as_str()).to_string(),
                or_dash(&item.review).to_string(),
                item.comments.to_string(),
                relations.join(","),
                item.title.clone(),
            ]);
        }
    }
    print!("{}", table(&rows));
    for error in errors {
        eprintln!("✘ {error}");
    }
}

/// Command-line flags as `--name value`, `--name=value` or a bare `--name`
/// for the names in `switches`.
struct Flags(Vec<(String, String)>);

impl Flags {
    fn parse(args: &[String], known: &[&str], switches: &[&str]) -> Result<Flags, String> {
        let mut values = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let Some(flag) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')) else {
                return Err(format!("argumento inesperado {arg:?}"));
            };
            let (name, inline) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (flag, None),
            };
            if !known.contains(&name) {
                return Err(format!("opção desconhecida --{name}"));
            }
            let value = match inline {
                Some(value) => value,
                None if switches.contains(&name) => "true".to_string(),
                None => args.next().cloned().ok_or_else(|| format!("--{name} exige um valor"))?,
            };
            values.push((name.to_string(), value));
        }
        Ok(Flags(values))
    }

    fn get(&self, name: &str) -> &str {
        self.0.iter().rev().find(|(flag, _)| flag == name).map_or("", |(_, value)| value)
    }

    fn switch(&self, name: &str) -> bool {
        matches!(self.get(name), "true" | "1")
    }
}

fn instance(mut cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            let mut rows = vec![["NOME", "PROVIDER", "HOST", "MERGE", "STATUS"].map(String::from).to_vec()];
            for instance in &cfg.instances {
                rows.push(vec![
                    instance.name.clone(),
                    instance.provider.to_string(),
                    instance.host.clone(),
                    or_dash(&instance.merge_method).to_string(),
                    if instance.disabled { "desativada" } else { "ativa" }.to_string(),
                ]);
            }
            print!("{}", table(&rows));
            Ok(())
        }
        Some("add") => {
            let known = ["provider", "host", "name", "merge-method", "auto-merge", "delete-branch"];
            let flags = Flags::parse(&args[1..], &known, &["auto-merge", "delete-branch"])?;
            let Some(provider) = Provider::parse(flags.get("provider")) else {
                return Err(format!("provider desconhecido {:?} (github, gitlab, gitea ou bitbucket)", flags.get("provider")));
            };
            let instance = Instance {
                name: flags.get("name").to_string(),
                provider,
                host: flags.get("host").to_string(),
                merge_method: flags.get("merge-method").to_string(),
                auto_merge: flags.switch("auto-merge"),
                delete_branch: flags.switch("delete-branch"),
                ..Instance::default()
            };
            cfg.upsert_instance("", instance).map_err(|err| err.to_string())?;
            cfg.save().map_err(|err| err.to_string())?;
            let Some(added) = cfg.instances.last() else { return Ok(()) };
            println!("instância {:?} adicionada ({} {})", added.name, added.provider, added.host);
            let client = provider::new(added, &[]);
            let ctx = Ctx::background().with_paths(&cfg.tool_paths).with_timeout(Duration::from_secs(20));
            if added.provider == Provider::Bitbucket {
                println!("aviso: a integração com Bitbucket ainda não existe — veja docs/bitbucket.md");
            } else if toolchain::lookup(&ctx, client.tool()).is_err() {
                println!("aviso: {} não está instalado. Instale: {}", client.tool(), provider::install_hint(client.tool()));
            } else if let Err(err) = client.auth_status(&ctx) {
                println!("aviso: {err}");
            }
            Ok(())
        }
        Some("rm" | "remove" | "delete") => {
            let Some(name) = args.get(1) else { return Err("uso: pr-tracker instance rm NOME".to_string()) };
            if !cfg.remove_instance(name) {
                return Err(format!("instância {name:?} não encontrada"));
            }
            cfg.save().map_err(|err| err.to_string())
        }
        Some(other) => Err(format!("subcomando desconhecido {other:?}")),
    }
}

fn repo(mut cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            let mut rows = vec![["INSTÂNCIA", "REPO", "PASTA", "REMOTE", "TODOS PRS/MRS"].map(String::from).to_vec()];
            for repo in &cfg.repos {
                rows.push(vec![
                    repo.instance.clone(),
                    repo.name.clone(),
                    repo.path.clone(),
                    or_dash(&repo.remote).to_string(),
                    if repo.track_all { "sim" } else { "não" }.to_string(),
                ]);
            }
            print!("{}", table(&rows));
            Ok(())
        }
        Some("set") => {
            let flags = Flags::parse(&args[1..], &["instance", "repo", "path", "remote", "merge-method"], &[])?;
            let (instance, name, path) = (flags.get("instance"), flags.get("repo"), flags.get("path"));
            if cfg.instance(instance).is_none() {
                return Err(format!("instância {instance:?} não encontrada"));
            }
            if name.is_empty() {
                return Err("--repo é obrigatório".to_string());
            }
            if !path.is_empty() {
                let ctx = Ctx::background().with_paths(&cfg.tool_paths);
                gitops::validate_clone(&ctx, &config::expand_home(path)).map_err(|err| err.to_string())?;
            }
            cfg.set_repo_path(instance, name, path);
            if let Some(repo) = cfg.repo_mut(instance, name) {
                repo.remote = flags.get("remote").to_string();
                repo.merge_method = flags.get("merge-method").to_string();
            }
            cfg.save().map_err(|err| err.to_string())
        }
        Some(other) => Err(format!("subcomando desconhecido {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn flags_accept_the_forms_the_go_cli_did() {
        let known = ["provider", "host", "auto-merge"];
        let flags = Flags::parse(&args(&["--provider", "gitlab", "-host=git.corp", "--auto-merge"]), &known, &["auto-merge"]).unwrap();
        assert_eq!((flags.get("provider"), flags.get("host"), flags.switch("auto-merge")), ("gitlab", "git.corp", true));
        assert_eq!(flags.get("name"), "");
        assert!(Flags::parse(&args(&["--unknown", "x"]), &known, &[]).is_err());
        assert!(Flags::parse(&args(&["--provider"]), &known, &[]).is_err());
        assert!(Flags::parse(&args(&["stray"]), &known, &[]).is_err());
    }

    #[test]
    fn table_pads_every_column_but_the_last() {
        let rows = vec![args(&["NOME", "HOST"]), args(&["trabalho", "git.corp"]), args(&["é", "h"])];
        assert_eq!(table(&rows), "NOME      HOST\ntrabalho  git.corp\né         h\n");
    }

    #[test]
    fn only_known_subcommands_bypass_the_window() {
        assert!(handles("doctor") && handles("--help") && handles("repos"));
        assert!(!handles("--snap") && !handles("--demo"));
    }
}
