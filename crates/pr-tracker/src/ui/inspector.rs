//! The selected item: its details, checks and conversation, and what can be
//! done to it.

use chrono::Utc;
use pr_tracker_core::app::Action;
use pr_tracker_core::config::Provider;
use pr_tracker_core::provider::{CiState, REVIEW_APPROVED, REVIEW_CHANGES_REQUESTED};
use rune::{El, FileDialogOptions, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Markdown, Menu, MenuItem, TextArea};

use super::list::Row;
use super::open_browser;
use super::state::{App, DetailTab};
use super::text::{relative_age, status_label};
use super::theme::Look;
use super::view::tab;
use super::widgets::{self, BannerKind, activity, banner, pill, rule, section, tinted};

fn has_integration(provider: Provider) -> bool {
    matches!(
        provider,
        Provider::GitHub | Provider::GitLab | Provider::Gitea
    )
}

fn markdown(source: &str, look: &Look) -> El {
    Markdown::new(source).selectable(true).build(&look.kit)
}

fn header(app: &App, selected: &Row, look: &Look) -> El {
    let k = &look.kit;
    let item = &selected.item;
    let busy = app.busy.contains_key(&selected.key);
    let glyph = if item.is_issue() {
        &widgets::ISSUE
    } else {
        &widgets::BRANCH
    };
    let mut reference = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(tinted(glyph, 14.0, look.accent))
        .child(
            text(selected.reference.clone())
                .font(k.ty.ui_medium)
                .color(look.muted),
        )
        .child(pill(
            app.tr(if item.draft { "Rascunho" } else { "Aberto" }),
            if item.draft { look.muted } else { look.green },
            look,
        ))
        .child(el().grow(1.0));
    if !item.is_issue() && has_integration(item.provider) {
        let (action, label) = if item.draft {
            (Action::MarkReady, "Marcar como Ready")
        } else {
            (Action::MarkDraft, "Marcar como Draft")
        };
        reference = reference.child(
            Button::new(app.tr(label))
                .variant(ButtonVariant::Ghost)
                .disabled(busy)
                .on_press(move |app: &mut App| app.ask(action, label, false, false, None))
                .build(k)
                .key("toggle-draft"),
        );
    }
    let mut header = col()
        .w_full()
        .gap(8.0)
        .child(reference)
        .child(
            text(item.title.clone())
                .key("inspector-title")
                .font(k.ty.body_medium)
                .color(look.text)
                .wrap()
                .selectable(true),
        )
        .child(
            text(format!("{} · {}", item.repo, item.author))
                .font(k.ty.ui)
                .color(look.muted)
                .truncate(),
        );
    if !item.is_issue() {
        let branch = |name: &str| {
            row()
                .px(6.0)
                .h(20.0)
                .center_y()
                .radius(4.0)
                .fill(look.raised)
                .shrink()
                .min_w(0.0)
                .child(
                    text(if name.is_empty() {
                        "—".to_string()
                    } else {
                        name.to_string()
                    })
                    .font(k.ty.mono)
                    .color(look.text)
                    .truncate(),
                )
        };
        let has_worktree = selected.worktree.is_some();
        let help = if has_worktree {
            "Buscar novos commits da branch de origem e atualizar o worktree local"
        } else {
            "Crie um worktree para atualizar a branch local"
        };
        header = header.child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(branch(&item.source_branch))
                .child(text("→").font(k.ty.ui).color(look.muted))
                .child(branch(&item.target_branch))
                .child(
                    Button::new(app.tr("Atualizar branch local"))
                        .variant(ButtonVariant::Ghost)
                        .disabled(busy || !has_worktree)
                        .help(app.tr(help))
                        .on_press(|app: &mut App| {
                            app.ask(
                                Action::UpdateWorktree,
                                "Atualizar branch local",
                                false,
                                false,
                                None,
                            )
                        })
                        .build(k)
                        .key("update-local-branch"),
                ),
        );
    }
    if !item.is_issue() && has_integration(item.provider) {
        let blocked = item.provider == Provider::GitHub && !item.can_update_branch;
        let help = if item.conflicts {
            "Resolva os conflitos antes de atualizar a branch"
        } else if blocked {
            "Branch atualizada ou sem permissão para atualizar no GitHub"
        } else {
            "Incorporar a branch base na origem do PR/MR no servidor"
        };
        let mut sync = row().w_full().gap(6.0).center_y().flex_wrap();
        if item.conflicts {
            sync = sync.child(
                pill(app.tr("Conflitos"), look.red, look)
                    .help(app.tr("Conflitos precisam ser resolvidos antes do merge.")),
            );
        }
        if item.behind_base {
            sync = sync.child(pill(app.tr("Branch desatualizada"), look.yellow, look));
        }
        header = header.child(
            sync.child(
                Button::new(app.tr("Atualizar com branch base"))
                    .variant(ButtonVariant::Outline)
                    .disabled(busy || item.conflicts || blocked)
                    .help(app.tr(help))
                    .on_press(|app: &mut App| {
                        app.ask(
                            Action::UpdateBranch,
                            "Atualizar com branch base",
                            false,
                            false,
                            None,
                        )
                    })
                    .build(k)
                    .key("update-branch"),
            ),
        );
    }
    if !item.labels.is_empty() {
        header = header.child(
            row().w_full().gap(4.0).flex_wrap().children(
                item.labels
                    .iter()
                    .map(|label| pill(label.clone(), look.blue, look)),
            ),
        );
    }
    header
}

fn details_tab(app: &App, selected: &Row, look: &Look) -> Vec<El> {
    let k = &look.kit;
    let item = &selected.item;
    let mut out = Vec::new();
    if let Some(thread) = &app.thread {
        let body = if thread.body.trim().is_empty() {
            app.tr("_Sem descrição._")
        } else {
            thread.body.clone()
        };
        out.push(markdown(&body, look).key("description"));
    }
    if item.is_issue() {
        return out;
    }
    let (review, ink) = match item.review.as_str() {
        REVIEW_APPROVED => ("✓ Aprovado", look.green),
        REVIEW_CHANGES_REQUESTED => ("Alterações solicitadas", look.red),
        _ => ("Aguardando revisão", look.muted),
    };
    let approvers = if item.approved_by.is_empty() {
        app.tr("Nenhuma aprovação informada")
    } else {
        item.approved_by.join(", ")
    };
    let mut reviews = section(app.tr("Revisões e aprovações"), look)
        .child(text(app.tr(review)).font(k.ty.ui_medium).color(ink))
        .child(text(approvers).font(k.ty.ui).color(look.muted).wrap());
    if item.conflicts {
        reviews = reviews.child(banner(
            "banner-conflicts",
            app.tr("Conflitos precisam ser resolvidos antes do merge."),
            BannerKind::Error,
            None,
            look,
        ));
    }
    out.push(reviews);
    let changed = (item.additions + item.deletions).max(1) as f32;
    out.push(
        section(
            format!("{} · {}", app.tr("Arquivos alterados"), item.files),
            look,
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    text(format!("+{}", item.additions))
                        .font(k.ty.ui_medium)
                        .color(look.green),
                )
                .child(
                    text(format!("−{}", item.deletions))
                        .font(k.ty.ui_medium)
                        .color(look.red),
                )
                .child(
                    row()
                        .grow(1.0)
                        .h(6.0)
                        .radius(3.0)
                        .fill(look.red.alpha(0.5))
                        .child(
                            el().w_frac(item.additions as f32 / changed)
                                .h(6.0)
                                .radius(3.0)
                                .fill(look.green),
                        ),
                ),
        ),
    );
    let worktree = selected
        .worktree
        .as_ref()
        .map(|dir| dir.display().to_string());
    let label = if worktree.is_some() {
        "Atualizar worktree"
    } else {
        "Criar worktree"
    };
    out.push(
        section("Worktree", look)
            .child(
                text(worktree.unwrap_or_else(|| app.tr("Nenhum worktree local")))
                    .font(k.ty.mono)
                    .color(look.muted)
                    .wrap_anywhere()
                    .selectable(true),
            )
            .child(
                Button::new(app.tr(label))
                    .variant(ButtonVariant::Outline)
                    .shortcut("w")
                    .disabled(app.busy.contains_key(&selected.key))
                    .on_press(move |app: &mut App| {
                        app.ask(Action::UpdateWorktree, label, false, false, None)
                    })
                    .build(k)
                    .key("worktree-button"),
            ),
    );
    out
}

fn checks_tab(app: &App, selected: &Row, look: &Look) -> Vec<El> {
    let k = &look.kit;
    let mut checks = section("CI / Checks", look);
    if selected.item.checks.is_empty() {
        checks = checks.child(
            text(app.tr("Nenhum check informado pelo provider."))
                .font(k.ty.ui)
                .color(look.muted)
                .wrap(),
        );
    }
    for check in &selected.item.checks {
        let (mark, ink) = match check.state {
            CiState::Success => ("✓", look.green),
            CiState::Failure => ("×", look.red),
            _ => ("◷", look.yellow),
        };
        checks = checks.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(mark).font(k.ty.ui_medium).color(ink).w(14.0))
                .child(
                    text(check.name.clone())
                        .font(k.ty.ui_medium)
                        .color(look.text)
                        .truncate()
                        .grow(1.0),
                )
                .child(
                    text(status_label(app.language, check.state.as_str()))
                        .font(k.ty.ui)
                        .color(look.muted),
                ),
        );
    }
    vec![checks]
}

fn conversation_tab(app: &App, selected: &Row, look: &Look) -> Vec<El> {
    let k = &look.kit;
    let mut out = Vec::new();
    if let Some(thread) = &app.thread {
        let body = if thread.body.trim().is_empty() {
            app.tr("_Sem descrição._")
        } else {
            thread.body.clone()
        };
        out.push(markdown(&body, look).key("description"));
        for (index, comment) in thread.comments.iter().enumerate() {
            let mut entry = col()
                .key(format!("comment-{index}"))
                .w_full()
                .gap(6.0)
                .padding(10.0)
                .radius(k.radius.md)
                .fill(look.raised)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(comment.author.clone())
                                .font(k.ty.ui_medium)
                                .color(look.text)
                                .truncate()
                                .grow(1.0),
                        )
                        .child(
                            text(relative_age(comment.created_at, Utc::now(), app.language))
                                .font(k.ty.small)
                                .color(look.muted),
                        ),
                );
            if !comment.review.is_empty() {
                let ink = match comment.review.as_str() {
                    REVIEW_APPROVED => look.green,
                    REVIEW_CHANGES_REQUESTED => look.red,
                    _ => look.muted,
                };
                entry = entry.child(row().child(pill(
                    status_label(app.language, &comment.review),
                    ink,
                    look,
                )));
            }
            if !comment.path.is_empty() {
                entry = entry.child(
                    text(format!("{}:{}", comment.path, comment.line))
                        .font(k.ty.mono)
                        .color(look.muted)
                        .wrap_anywhere(),
                );
            }
            out.push(entry.child(markdown(&comment.body, look)));
        }
    }
    let busy = app.busy.contains_key(&selected.key);
    out.push(
        col()
            .key("composer")
            .w_full()
            .gap(6.0)
            .pt(8.0)
            .child(
                text(app.tr("Adicionar comentário"))
                    .font(k.ty.ui_medium)
                    .color(look.text),
            )
            .child(
                TextArea::new(&app.draft)
                    .label(app.tr("Adicionar comentário"))
                    .placeholder(app.tr("Escreva em Markdown…"))
                    .auto_grow(4, 12)
                    .on_edit(|app: &mut App| &mut app.draft)
                    .build(k)
                    .key("comment"),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(app.tr("Rascunho salvo neste dispositivo"))
                            .font(k.ty.small)
                            .color(look.muted)
                            .truncate()
                            .grow(1.0),
                    )
                    .child(
                        Button::new(app.tr("Revisar envio →"))
                            .variant(ButtonVariant::Contrast)
                            .disabled(busy || app.draft.text.trim().is_empty())
                            .on_press(|app: &mut App| {
                                app.ask(Action::Comment, "Enviar comentário", false, false, None)
                            })
                            .build(k)
                            .key("send-comment"),
                    ),
            ),
    );
    out
}

fn more_menu(app: &App, selected: &Row, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let item = &selected.item;
    let busy = app.busy.contains_key(&selected.key);
    let ask = |label: &'static str, action: Action, remove_after: bool| {
        MenuItem::new(app.tr(label))
            .disabled(busy)
            .on_select(move |app: &mut App| app.ask(action, label, false, remove_after, None))
    };
    let trigger = Button::new(app.tr("Mais ações"))
        .variant(ButtonVariant::Outline)
        .expanded(app.more_open)
        .on_press(|app: &mut App| app.more_open = !app.more_open)
        .build(k)
        .key("more-actions");
    let mut menu = Menu::new("more-menu")
        .min_width(320.0)
        .trigger(trigger)
        .open(app.more_open)
        .on_close(|app: &mut App| app.more_open = false)
        .item(
            MenuItem::new(app.tr("Abrir no navegador"))
                .shortcut("o")
                .on_select_tx(open_browser),
        )
        .item(
            MenuItem::new(app.tr("Abrir pasta local"))
                .disabled(selected.clone.is_none() && selected.worktree.is_none())
                .on_select(|app: &mut App| app.open_folder()),
        );
    if !item.is_issue() {
        let noun = if item.provider == Provider::GitLab {
            "MRs"
        } else {
            "PRs"
        };
        let track = if app.tracks_all() {
            "Parar de acompanhar todos os "
        } else {
            "Acompanhar todos os "
        };
        let key = selected.key.clone();
        menu = menu
            .item(ask("Abrir terminal", Action::PrepareTerminal, false).shortcut("t"))
            .item(ask("Ver diff", Action::PrepareDiff, false).shortcut("d"))
            .item(ask("Checkout no clone", Action::Checkout, false).shortcut("c"))
            .separator()
            .item(
                MenuItem::new(format!(
                    "{}{noun}{}",
                    app.tr(track),
                    app.tr(" deste repositório")
                ))
                .disabled(app.settings_busy())
                .on_select(|app: &mut App| app.toggle_track_all()),
            )
            .item(
                MenuItem::new(app.tr("Definir pasta local do repositório"))
                    .disabled(app.settings_busy())
                    .on_select_tx(move |app: &mut App, tx: &mut Tx| {
                        app.more_open = false;
                        let key = key.clone();
                        let options = FileDialogOptions {
                            title: app.tr("Selecionar clone local"),
                            ..FileDialogOptions::default()
                        };
                        tx.open_folder(options, move |app: &mut App, result, _| match result {
                            Ok(Some(path)) => app.set_repo_folder(&key, &path),
                            Ok(None) => {}
                            Err(err) => app.error = err.to_string(),
                        });
                    }),
            )
            .separator()
            .item(ask("Fechar sem merge", Action::Close, false).danger(true));
        if selected.worktree.is_some() {
            menu = menu
                .item(ask("Aprovar e remover worktree", Action::Approve, true))
                .item(ask("Merge e remover worktree", Action::Merge, true))
                .item(ask("Fechar sem merge e remover worktree", Action::Close, true).danger(true))
                .item(
                    ask("Remover worktree", Action::RemoveWorktree, false)
                        .shortcut("x")
                        .danger(true),
                );
        }
    }
    menu.build(k, layer)
}

fn actions(app: &App, selected: &Row, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let busy = app.busy.contains_key(&selected.key);
    let mut bar = row()
        .key("detail-actions")
        .w_full()
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y();
    if !selected.item.is_issue() {
        bar = bar
            .child(
                Button::new(app.tr("✓ Aprovar"))
                    .variant(ButtonVariant::Submit)
                    .shortcut("a")
                    .disabled(busy)
                    .on_press(|app: &mut App| {
                        app.ask(Action::Approve, "Aprovar pull request", false, false, None)
                    })
                    .build(k)
                    .key("action-approve"),
            )
            .child(
                Button::new("Merge")
                    .variant(ButtonVariant::Contrast)
                    .shortcut("m")
                    .disabled(busy)
                    .on_press(|app: &mut App| {
                        app.ask(Action::Merge, "Fazer merge", false, false, None)
                    })
                    .build(k)
                    .key("action-merge"),
            );
    }
    bar.child(el().grow(1.0))
        .child(more_menu(app, selected, look, layer))
}

/// The pane beside the list. Overlays it opens are pushed onto `layer`.
pub fn inspector(app: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let Some(selected) = app.selected_row() else {
        return col()
            .key("inspector-empty")
            .w_full()
            .h_full()
            .center()
            .gap(8.0)
            .padding(24.0)
            .child(tinted(&widgets::BRANCH, 24.0, look.muted))
            .child(
                text(app.tr("Um pouco de contexto"))
                    .font(k.ty.body_medium)
                    .color(look.text),
            )
            .child(
                text(app.tr("Selecione um item para ver detalhes, checks e conversa."))
                    .font(k.ty.ui)
                    .color(look.muted)
                    .wrap(),
            );
    };
    let item = &selected.item;
    let detail_tabs = row()
        .key("detail-tabs")
        .w_full()
        .gap(4.0)
        .child(tab(
            "tab-details".into(),
            app.tr("Detalhes"),
            None,
            app.detail_tab == DetailTab::Details,
            look,
            |app| app.detail_tab = DetailTab::Details,
        ))
        .child(tab(
            "tab-checks".into(),
            "Checks".into(),
            Some(item.checks.len()),
            app.detail_tab == DetailTab::Checks,
            look,
            |app| app.detail_tab = DetailTab::Checks,
        ))
        .child(tab(
            "tab-conversation".into(),
            app.tr("Conversa"),
            Some(item.comments.max(0) as usize),
            app.detail_tab == DetailTab::Conversation,
            look,
            |app| app.detail_tab = DetailTab::Conversation,
        ));
    let mut body = col()
        .key("detail-body")
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .child(header(app, selected, look))
        .child(detail_tabs);
    let presets = app.comment_presets();
    if !presets.is_empty() {
        let busy = app.busy.contains_key(&selected.key);
        let buttons = presets.iter().enumerate().map(|(index, preset)| {
            let text = preset.body.clone();
            Button::new(preset.label.clone())
                .variant(ButtonVariant::Outline)
                .disabled(busy)
                .on_press(move |app: &mut App| {
                    app.ask(
                        Action::Comment,
                        "Enviar comentário",
                        false,
                        false,
                        Some(text.clone()),
                    )
                })
                .build(k)
                .key(format!("preset-{index}"))
        });
        body = body.child(
            col()
                .key("comment-presets")
                .w_full()
                .gap(6.0)
                .child(
                    text(app.tr("Comentários predefinidos"))
                        .font(k.ty.small_medium)
                        .color(look.muted),
                )
                .child(row().w_full().gap(6.0).flex_wrap().children(buttons)),
        );
    }
    if app.detail_loading {
        body = body.child(activity(app.tr("Carregando conversa…"), look).key("detail-loading"));
    }
    if !app.detail_error.is_empty() {
        let retry = selected.key.clone();
        body = body
            .child(banner(
                "banner-detail",
                app.detail_error.clone(),
                BannerKind::Error,
                None,
                look,
            ))
            .child(
                row().child(
                    Button::new(app.tr("Tentar novamente"))
                        .on_press(move |app: &mut App| app.select(&retry))
                        .build(k)
                        .key("detail-retry"),
                ),
            );
    }
    body = body.children(match app.detail_tab {
        DetailTab::Details => details_tab(app, selected, look),
        DetailTab::Checks => checks_tab(app, selected, look),
        DetailTab::Conversation => conversation_tab(app, selected, look),
    });
    col()
        .key("inspector")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            body.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        )
        .child(rule(look))
        .child(actions(app, selected, look, layer))
}
