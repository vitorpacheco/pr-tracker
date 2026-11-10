//! Fictional data for `--demo`: the window without accounts, network or side effects.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use pr_tracker_core::app::{Application, Hooks};
use pr_tracker_core::config::{CommentPreset, Config, Instance, Provider};
use pr_tracker_core::provider::{
    Check, CiState, Client, Comment, Item, Kind, MergeOptions, REVIEW_APPROVED, REVIEW_REQUIRED,
    Relation, Thread,
};
use pr_tracker_core::toolchain::Ctx;
use pr_tracker_core::{Error, Result, msg};

const ROWS: [(&str, i64, &str, &str, CiState, Relation); 8] = [
    (
        "platform/web",
        482,
        "Adiciona autenticação via token para API interna",
        "mrlima",
        CiState::Success,
        Relation::REVIEW_REQUESTED,
    ),
    (
        "services/auth",
        317,
        "Refatora fluxo de renovação de sessão",
        "jcarvalho",
        CiState::Pending,
        Relation::REVIEW_REQUESTED,
    ),
    (
        "platform/cli",
        219,
        "Melhora cache do pipeline e reduz tempo de build",
        "fernandes",
        CiState::Success,
        Relation::REVIEW_REQUESTED,
    ),
    (
        "ui/components",
        741,
        "Novo componente de tabela com virtualização",
        "anacosta",
        CiState::Pending,
        Relation::AUTHORED,
    ),
    (
        "services/payments",
        603,
        "Corrige tratamento de falha no webhook",
        "dlima",
        CiState::Failure,
        Relation::ASSIGNED,
    ),
    (
        "platform/cli",
        518,
        "Adiciona comando para gerenciar worktrees",
        "rbarbosa",
        CiState::Success,
        Relation::REVIEW_REQUESTED,
    ),
    (
        "docs",
        402,
        "Atualiza documentação de contribuição",
        "cferreira",
        CiState::None,
        Relation::AUTHORED,
    ),
    (
        "infra/k8s",
        288,
        "Ajusta probes de readiness e liveness",
        "mrlima",
        CiState::Success,
        Relation::ASSIGNED,
    ),
];

const INSTANCES: [&str; 2] = ["Work Git", "Corp Git"];
const WORKTREE: &str = "/example/worktree";

pub fn config() -> Config {
    let mut cfg = Config::default();
    let preset = |label: &str, body: &str| CommentPreset {
        label: label.into(),
        body: body.into(),
    };
    cfg.instances = vec![
        Instance {
            name: INSTANCES[0].into(),
            provider: Provider::GitHub,
            host: "github.com".into(),
            merge_method: "squash".into(),
            comment_presets: vec![
                preset("Aprovado", "aprovado"),
                preset("Reprovado", "reprovado"),
            ],
            ..Instance::default()
        },
        Instance {
            name: INSTANCES[1].into(),
            provider: Provider::GitHub,
            host: "git.corp.example".into(),
            ..Instance::default()
        },
    ];
    cfg
}

fn items(instance: &Instance) -> Vec<Item> {
    let now = Utc::now();
    let check = |name: &str, state| Check {
        name: name.into(),
        state,
        url: String::new(),
    };
    let mut items: Vec<Item> = ROWS
        .iter()
        .enumerate()
        .filter(|(index, _)| INSTANCES[usize::from(index % 3 != 0)] == instance.name)
        .map(
            |(index, &(repo, number, title, author, ci, relations))| Item {
                kind: Kind::Pr,
                instance: instance.name.clone(),
                provider: instance.provider,
                host: instance.host.clone(),
                repo: repo.into(),
                number,
                title: title.into(),
                url: "https://example.com".into(),
                author: author.into(),
                draft: index == 3,
                source_branch: "feature/token-auth".into(),
                target_branch: "main".into(),
                behind_base: index == 0,
                can_update_branch: index == 0,
                created_at: Some(now - Duration::days(1)),
                updated_at: Some(now - Duration::minutes(12 * (index as i64 + 1))),
                additions: 328,
                deletions: 42,
                files: 12,
                review: if index == 0 {
                    REVIEW_APPROVED
                } else {
                    REVIEW_REQUIRED
                }
                .into(),
                approved_by: if index == 0 {
                    vec!["jcarvalho".into(), "anacosta".into()]
                } else {
                    Vec::new()
                },
                conflicts: index == 1,
                ci,
                checks: vec![
                    check("build", CiState::Success),
                    check("test", CiState::Success),
                    check("lint", ci),
                ],
                labels: vec!["enhancement".into(), "security".into()],
                comments: index as i64 + 2,
                relations,
                ..Item::default()
            },
        )
        .collect();
    if instance.name == INSTANCES[0] {
        items.push(Item {
            kind: Kind::Issue,
            instance: instance.name.clone(),
            provider: instance.provider,
            host: instance.host.clone(),
            repo: "platform/web".into(),
            number: 88,
            title: "Documentar configuração dos providers".into(),
            url: "https://example.com".into(),
            author: "mrlima".into(),
            updated_at: Some(now - Duration::hours(3)),
            comments: 2,
            relations: Relation::ASSIGNED,
            ..Item::default()
        });
    }
    items
}

struct DemoClient {
    instance: Instance,
}

fn read_only() -> Error {
    msg!("Demonstração: esta ação não altera dados.")
}

impl Client for DemoClient {
    fn instance(&self) -> &Instance {
        &self.instance
    }
    fn tool(&self) -> &'static str {
        "demo"
    }
    fn list(&self, _: &Ctx) -> Result<Vec<Item>> {
        Ok(items(&self.instance))
    }
    fn thread(&self, _: &Ctx, _: &Item) -> Result<Thread> {
        Ok(Thread {
            body: "Implementa autenticação via **Bearer token** para os endpoints da API interna.\n\n- Validação de escopos\n- Middleware de segurança\n- Cobertura de testes\n\n```go\nrequest.Header.Set(\"Authorization\", \"Bearer \" + token)\n```".into(),
            comments: vec![Comment {
                author: "jcarvalho".into(),
                body: "A implementação ficou clara. Os testes de escopo estão passando.".into(),
                created_at: Some(Utc::now() - Duration::minutes(40)),
                review: REVIEW_APPROVED.into(),
                ..Comment::default()
            }],
        })
    }
    fn add_comment(&self, _: &Ctx, _: &Item, _: &str) -> Result<()> {
        Err(read_only())
    }
    fn approve(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(read_only())
    }
    fn merge(&self, _: &Ctx, _: &Item, _: &MergeOptions) -> Result<()> {
        Err(read_only())
    }
    fn close(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(read_only())
    }
    fn set_draft(&self, _: &Ctx, _: &Item, _: bool) -> Result<()> {
        Err(read_only())
    }
    fn update_branch(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(read_only())
    }
    fn checkout(&self, _: &Ctx, _: &Item, _: &Path) -> Result<()> {
        Err(read_only())
    }
    fn head_ref(&self, pr: &Item) -> String {
        format!("refs/pull/{}/head", pr.number)
    }
    fn auth_status(&self, _: &Ctx) -> Result<()> {
        Ok(())
    }
}

/// An application whose providers and worktrees are canned.
pub fn application() -> Application {
    Application::new(&config(), None).with_hooks(Hooks {
        client: Arc::new(|instance, _| {
            Arc::new(DemoClient {
                instance: instance.clone(),
            })
        }),
        resolve_clone: Arc::new(|_, _, _| None),
        worktree_exists: Arc::new(|_, item| {
            (item.number == ROWS[0].1).then(|| PathBuf::from(WORKTREE))
        }),
        create_worktree: Arc::new(|_, _, _, _, _, _| Err(read_only())),
        push_worktree: Arc::new(|_, _, _| Err(read_only())),
        remove_worktree: Arc::new(|_, _, _, _| Err(read_only())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pr_tracker_core::app::{Action, Command, Session};

    #[test]
    fn demo_session_lists_items_and_refuses_changes() {
        let session = Session::with_application(application());
        let state = session.refresh(&Ctx::background()).unwrap();
        assert_eq!(state.items.len(), ROWS.len() + 1);
        assert_eq!(state.worktrees.len(), 1);
        let key = state
            .items
            .iter()
            .find(|item| item.number == 482)
            .unwrap()
            .key();
        assert!(
            session
                .detail(&Ctx::background(), &key)
                .unwrap()
                .body
                .contains("Bearer token")
        );
        let out = session.execute(&Ctx::background(), &key, Command::new(Action::Approve));
        assert_eq!(
            out.error.map(|err| err.text("en")),
            Some("Demo: this action does not change data.".to_string())
        );
        session.close();
    }
}
