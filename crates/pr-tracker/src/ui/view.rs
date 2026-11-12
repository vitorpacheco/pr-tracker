//! The window's frame: toolbar, sidebar and the list of items.

use chrono::Utc;
use pr_tracker_core::provider::{
    CiState, Kind, REVIEW_APPROVED, REVIEW_CHANGES_REQUESTED, Relation,
};
use rune::{Cursor, Cx, El, ScrollSpec, Tx, col, row, text};
use rune_kit::shell::{Sidebar, SidebarRow, SidebarSection, SplitView, Toolbar};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, Spinner, TextInput};

use super::list::{self, Row};
use super::state::App;
use super::text::{relative_age, status_label};
use super::theme::{self, Look};
use super::widgets::{
    self, BannerKind, activity, avatar, banner, eyebrow, kbd, lifted, pill, tinted,
};
use super::{dialogs, inspector};

/// The element key of an item's row.
pub fn row_key(key: &str) -> String {
    format!("row-{key}")
}

/// The views of each kind: label (a catalog key) and relation.
pub fn tabs(kind: Kind) -> [(&'static str, Relation); 4] {
    match kind {
        Kind::Pr => [
            ("Todos", Relation::NONE),
            ("Revisar", Relation::REVIEW_REQUESTED),
            ("Meus", Relation::AUTHORED),
            ("Atribuídos", Relation::ASSIGNED),
        ],
        Kind::Issue => [
            ("Todas", Relation::NONE),
            ("Atribuídas", Relation::ASSIGNED),
            ("Criadas", Relation::AUTHORED),
            ("Menções", Relation::MENTIONED),
        ],
    }
}

pub fn tab(
    key: String,
    label: String,
    count: Option<usize>,
    active: bool,
    look: &Look,
    press: impl Fn(&mut App) + 'static,
) -> El {
    let ink = if active { look.text } else { look.muted };
    let mut tab = row()
        .key(key)
        .role("tab")
        .label(label.clone())
        .checked(active)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(look.kit.radius.md)
        .fill(if active {
            look.selection
        } else {
            look.panel.alpha(0.0)
        })
        .hover(look.kit.c.hover)
        .on_press(press)
        .child(text(label).font(look.kit.ty.ui_medium).color(ink));
    if let Some(count) = count {
        tab = tab.child(
            text(count.to_string())
                .font(look.kit.ty.small)
                .color(look.muted),
        );
    }
    tab
}

fn toolbar(app: &App, look: &Look) -> Toolbar {
    let k = &look.kit;
    let kinds = row()
        .key("kind-tabs")
        .gap(2.0)
        .child(tab(
            "kind-prs".into(),
            "PRs".into(),
            None,
            app.kind == Kind::Pr,
            look,
            |app| app.switch_kind(Kind::Pr),
        ))
        .child(tab(
            "kind-issues".into(),
            "Issues".into(),
            None,
            app.kind == Kind::Issue,
            look,
            |app| app.switch_kind(Kind::Issue),
        ));
    let search = TextInput::new(&app.query)
        .placeholder(app.tr("Buscar título, repo, autor…"))
        .label(app.tr("Buscar itens"))
        .leading_icon(k.glyph.search)
        .clear_button()
        .width(Some(320.0))
        .on_edit(|app: &mut App| &mut app.query)
        .on_change(|app: &mut App, _: &mut Tx| app.reselect())
        .build(k)
        .key("search");
    let busy = app.loading || app.refreshing;
    let refresh = if busy {
        row()
            .key("refreshing")
            .size(28.0, 28.0)
            .center()
            .child(Spinner::new().size(14.0).color(look.accent).build(k))
    } else {
        IconButton::new(&widgets::REFRESH)
            .variant(IconButtonVariant::GhostMuted)
            .label(app.tr("Atualizar"))
            .help(app.tr("Atualizar (r)"))
            .on_press(|app: &mut App| app.refresh())
            .build(k)
            .key("refresh")
    };
    let commands = IconButton::new(&widgets::COMMAND)
        .variant(IconButtonVariant::GhostMuted)
        .label(app.tr("Abrir comandos"))
        .help(app.tr("Comandos"))
        .on_press(|app: &mut App| app.toggle_palette())
        .build(k)
        .key("open-palette");
    let settings = IconButton::new(&widgets::GEAR)
        .variant(IconButtonVariant::GhostMuted)
        .label(app.tr("Configurações"))
        .help(app.tr("Configurações"))
        .disabled(app.settings_busy())
        .on_press(|app: &mut App| app.open_settings())
        .build(k)
        .key("open-settings");
    let inspector = IconButton::new(&rune_kit::shell::INSPECTOR)
        .variant(IconButtonVariant::GhostMuted)
        .expanded(app.inspector.is_open())
        .label(app.tr("Detalhes do item"))
        .on_press(|app: &mut App| app.inspector.toggle())
        .build(k)
        .key("toggle-inspector");
    Toolbar::new().leading(kinds).center(search).trailing(
        row()
            .gap(2.0)
            .center_y()
            .child(commands)
            .child(refresh)
            .child(settings)
            .child(inspector),
    )
}

fn sidebar(app: &App, look: &Look) -> El {
    let k = &look.kit;
    let all = SidebarRow::new(app.tr("Todas as instâncias"))
        .detail(app.rows.len().to_string())
        .selected(app.instance.is_empty())
        .on_select(|app: &mut App, _| {
            app.instance.clear();
            app.reselect();
        })
        .build(k)
        .key("instance-all");
    let mut instances = SidebarSection::new(app.tr("INSTÂNCIAS")).key("instances");
    for instance in app.instances() {
        let name = instance.name.clone();
        let total = app
            .rows
            .iter()
            .filter(|row| row.item.instance == name)
            .count();
        let mut entry = SidebarRow::new(name.clone())
            .detail(total.to_string())
            .selected(app.instance == name);
        if app.errors.contains_key(&name) {
            entry = entry.trailing(pill("!", look.red, look));
        }
        let chosen = name.clone();
        instances = instances.row(
            entry
                .on_select(move |app: &mut App, _| app.set_instance(&chosen))
                .build(k)
                .key(format!("instance-{name}")),
        );
    }
    instances = instances.row(
        SidebarRow::new(app.tr("＋ Adicionar instância"))
            .on_select(|app: &mut App, _| app.open_settings())
            .build(k)
            .key("instance-add"),
    );
    let mut views = SidebarSection::new(app.tr("VISÕES")).key("views");
    for (label, relation) in tabs(app.kind) {
        views = views.row(
            SidebarRow::new(app.tr(label))
                .detail(app.count(app.kind, relation).to_string())
                .selected(app.relation == relation)
                .on_select(move |app: &mut App, _| app.set_relation(relation))
                .build(k)
                .key(format!("view-{}", relation.0)),
        );
    }
    let synced = match app.synced_at {
        Some(at) => format!(
            "{}{}",
            app.tr("Sincronizado "),
            relative_age(Some(at), Utc::now(), app.language)
        ),
        None => app.tr("Aguardando sincronização"),
    };
    let source = if app.demo {
        app.tr("Demonstração · dados fictícios")
    } else {
        app.tr("Cache local · SQLite")
    };
    let footer = col()
        .key("sync-status")
        .w_full()
        .px(8.0)
        .py(8.0)
        .gap(2.0)
        .child(
            text(source)
                .font(k.ty.small_medium)
                .color(look.muted)
                .truncate(),
        )
        .child(text(synced).font(k.ty.small).color(look.muted).truncate());
    Sidebar::new()
        .label(app.tr("Menu lateral"))
        .header(all)
        .section(instances)
        .section(views)
        .footer(footer)
        .build(k)
}

fn notifications(app: &App, look: &Look) -> Vec<El> {
    let mut out = Vec::new();
    if app.loading || app.refreshing {
        let label = if app.loading {
            "Carregando sua fila"
        } else {
            "Sincronizando itens…"
        };
        out.push(activity(app.tr(label), look).key("activity-sync"));
    }
    for (name, label) in &app.pending {
        out.push(activity(label.clone(), look).key(format!("activity-{name}")));
    }
    for (key, label) in &app.busy {
        out.push(activity(label.clone(), look).key(format!("activity-{key}")));
    }
    if !app.theme_error.is_empty() {
        out.push(banner(
            "banner-theme",
            app.theme_error.clone(),
            BannerKind::Error,
            None,
            look,
        ));
    }
    if !app.error.is_empty() && app.error != app.theme_error {
        out.push(banner(
            "banner-error",
            app.error.clone(),
            BannerKind::Error,
            Some(|app| app.error.clear()),
            look,
        ));
    }
    if !app.notice.is_empty() {
        out.push(banner(
            "banner-notice",
            app.notice.clone(),
            BannerKind::Notice,
            Some(|app| app.notice.clear()),
            look,
        ));
    }
    if app.demo {
        out.push(banner(
            "banner-demo",
            app.tr("Demonstração visual — nenhuma ação altera dados."),
            BannerKind::Notice,
            None,
            look,
        ));
    }
    if !app.cache_error.is_empty() {
        out.push(banner(
            "banner-cache",
            format!("{} {}", app.tr("Cache indisponível:"), app.cache_error),
            BannerKind::Error,
            None,
            look,
        ));
    }
    for (name, message) in &app.errors {
        let text = format!(
            "{name}: {message} {}",
            app.tr("· exibindo dados anteriores.")
        );
        out.push(banner(
            &format!("banner-instance-{name}"),
            text,
            BannerKind::Error,
            None,
            look,
        ));
    }
    out
}

fn ci_mark(state: CiState, look: &Look) -> (&'static str, rune::Color) {
    match state {
        CiState::Success => ("✓", look.green),
        CiState::Failure => ("×", look.red),
        CiState::Pending => ("◷", look.yellow),
        CiState::Canceled => ("⊘", look.muted),
        CiState::None => ("—", look.muted),
    }
}

fn item_row(app: &App, row_data: &Row, look: &Look) -> El {
    let k = &look.kit;
    let item = &row_data.item;
    let selected = app.selected == row_data.key;
    let glyph = if item.is_issue() {
        &widgets::ISSUE
    } else {
        &widgets::BRANCH
    };
    let mut line = row().gap(6.0).center_y().child(tinted(
        glyph,
        14.0,
        if item.draft { look.muted } else { look.accent },
    ));
    if app.kind == Kind::Issue {
        line = line.child(
            text(item.repo.clone())
                .font(k.ty.small)
                .color(look.muted)
                .truncate(),
        );
    }
    line = line.child(
        text(row_data.reference.clone())
            .font(k.ty.small_medium)
            .color(look.muted),
    );
    if app.busy.contains_key(&row_data.key) {
        line = line.child(Spinner::new().size(12.0).color(look.accent).build(k));
    }
    if item.draft {
        line = line.child(pill(app.tr("Rascunho"), look.muted, look));
    }
    if item.conflicts {
        line = line.child(
            pill(app.tr("Conflitos"), look.red, look)
                .help(app.tr("Conflitos precisam ser resolvidos antes do merge.")),
        );
    }
    if !item.is_issue() {
        let review = match item.review.as_str() {
            REVIEW_APPROVED => Some(("✓ Aprovado", look.green)),
            REVIEW_CHANGES_REQUESTED => Some(("Alterações solicitadas", look.red)),
            _ => None,
        };
        if let Some((label, ink)) = review {
            line = line.child(pill(app.tr(label), ink, look));
        }
        if item.additions + item.deletions > 0 {
            line = line
                .child(
                    text(format!("+{}", item.additions))
                        .font(k.ty.small)
                        .color(look.green),
                )
                .child(
                    text(format!("−{}", item.deletions))
                        .font(k.ty.small)
                        .color(look.red),
                );
        }
    }
    let title_ink = if selected {
        look.selection_text
    } else {
        look.text
    };
    let main = col().grow(1.0).min_w(0.0).gap(3.0).child(line).child(
        text(item.title.clone())
            .font(k.ty.ui_medium)
            .color(title_ink)
            .truncate(),
    );
    let author = row()
        .w(140.0)
        .gap(8.0)
        .center_y()
        .child(avatar(&item.author, look))
        .child(
            text(item.author.clone())
                .font(k.ty.ui)
                .color(look.muted)
                .truncate()
                .grow(1.0),
        );
    let (mark, ink) = ci_mark(item.ci, look);
    let ci_help = if item.ci == CiState::None {
        app.tr("Sem checks")
    } else {
        status_label(app.language, item.ci.as_str())
    };
    let ci = row()
        .w(40.0)
        .center()
        .child(text(mark).font(k.ty.ui_medium).color(ink))
        .help(ci_help);
    let updated = col()
        .w(96.0)
        .items_end()
        .gap(2.0)
        .child(
            text(relative_age(item.updated_at, Utc::now(), app.language))
                .font(k.ty.small)
                .color(look.muted),
        )
        .child(
            row()
                .gap(4.0)
                .center_y()
                .child(tinted(&widgets::COMMENT, 12.0, look.muted))
                .child(
                    text(item.comments.to_string())
                        .font(k.ty.small)
                        .color(look.muted),
                ),
        );
    let key = row_data.key.clone();
    let surface = row()
        .key(row_key(&row_data.key))
        .role("row")
        .label(format!(
            "{} {}: {}",
            item.repo, row_data.reference, item.title
        ))
        .checked(selected)
        .cursor(Cursor::Pointer)
        .w_full()
        .px(12.0)
        .py(9.0)
        .gap(12.0)
        .center_y()
        .radius(k.radius.md)
        .fill(if selected {
            look.selection
        } else {
            look.panel.alpha(0.0)
        })
        .hover(k.c.hover)
        .on_press(move |app: &mut App| {
            if app.selected != key {
                app.select(&key);
            }
            if !app.inspector.is_open() {
                app.inspector.toggle();
            }
        })
        .child(main)
        .child(author)
        .child(ci)
        .child(updated);
    if selected {
        lifted(surface, look)
    } else {
        surface
    }
}

fn empty(app: &App, look: &Look) -> El {
    let k = &look.kit;
    if app.loading {
        return col()
            .key("list-loading")
            .w_full()
            .py(64.0)
            .gap(10.0)
            .items_center()
            .child(Spinner::new().size(20.0).color(look.accent).build(k))
            .child(
                text(app.tr("Carregando sua fila"))
                    .font(k.ty.body_medium)
                    .color(look.text),
            )
            .child(
                text(app.tr("Buscando primeiro os dados do cache local."))
                    .font(k.ty.ui)
                    .color(look.muted),
            );
    }
    let any = !app.rows.is_empty();
    let title = if app.query.text.trim().is_empty() {
        "Tudo em dia por aqui"
    } else {
        "Nenhum resultado"
    };
    let hint = if any {
        "Experimente outra visão ou busca."
    } else {
        "Configure uma instância ou atualize para buscar seus PRs e issues."
    };
    let action = if any {
        Button::new(app.tr("Limpar filtros"))
            .on_press(|app: &mut App| app.clear_filters())
            .build(k)
            .key("clear-filters")
    } else {
        Button::new(app.tr("Configurar instâncias"))
            .on_press(|app: &mut App| app.open_settings())
            .build(k)
            .key("configure-instances")
    };
    col()
        .key("list-empty")
        .w_full()
        .py(64.0)
        .gap(10.0)
        .items_center()
        .child(text("✓").font(k.ty.body_medium).color(look.green))
        .child(text(app.tr(title)).font(k.ty.body_medium).color(look.text))
        .child(text(app.tr(hint)).font(k.ty.ui).color(look.muted).wrap())
        .child(action)
}

fn list_pane(app: &App, look: &Look) -> El {
    let k = &look.kit;
    let visible = app.visible();
    let heading = col()
        .w_full()
        .gap(4.0)
        .child(eyebrow(app.tr("SUA FILA DE TRABALHO"), look))
        .child(
            row()
                .gap(10.0)
                .items_end()
                .child(
                    text(if app.kind == Kind::Pr {
                        "Pull requests"
                    } else {
                        "Issues"
                    })
                    .font(k.ty.body_medium)
                    .color(look.text),
                )
                .child(
                    text(visible.len().to_string())
                        .key("visible-count")
                        .font(k.ty.ui)
                        .color(look.muted),
                ),
        )
        .child(
            text(format!(
                "{} · {}",
                if app.instance.is_empty() {
                    app.tr("Todas as instâncias")
                } else {
                    app.instance.clone()
                },
                app.tr(if app.kind == Kind::Pr {
                    "por repositório · mais recentes primeiro"
                } else {
                    "mais recentes primeiro"
                })
            ))
            .font(k.ty.small)
            .color(look.muted),
        );
    // The sidebar lists the same views, so the tabs only stand in for it
    // while it is collapsed.
    let mut filters = row().key("filters").gap(4.0).flex_wrap();
    let views = if app.sidebar.is_open() {
        Vec::new()
    } else {
        tabs(app.kind).to_vec()
    };
    for (label, relation) in views {
        filters = filters.child(tab(
            format!("filter-{}", relation.0),
            app.tr(label),
            Some(app.count(app.kind, relation)),
            app.relation == relation,
            look,
            move |app| app.set_relation(relation),
        ));
    }
    let head = |label: String, width: Option<f32>| {
        let cell = text(label).font(k.ty.small_medium).color(look.muted);
        match width {
            Some(width) => cell.w(width),
            None => cell.grow(1.0),
        }
    };
    let table_head = row()
        .w_full()
        .px(12.0)
        .gap(12.0)
        .child(head(app.tr("REPOSITÓRIO / TÍTULO"), None))
        .child(head(app.tr("AUTOR"), Some(140.0)))
        .child(head("CI".into(), Some(40.0)))
        .child(head(app.tr("ATUALIZADO ↓"), Some(96.0)));
    let mut rows = col().key("rows").w_full().gap(2.0).pb(12.0);
    if visible.is_empty() {
        rows = rows.child(empty(app, look));
    } else if app.kind == Kind::Pr {
        for group in list::group_by_repository(&visible) {
            // A filled bar, so the repository reads as a heading for the
            // rows under it instead of as one more line of the list.
            rows = rows.child(
                col().w_full().pt(10.0).pb(2.0).child(
                    row()
                        .key(format!("group-{}-{}", group.instance, group.repo))
                        .w_full()
                        .px(12.0)
                        .py(7.0)
                        .gap(8.0)
                        .center_y()
                        .radius(k.radius.md)
                        .fill(look.raised)
                        .border(1.0, look.border)
                        .child(tinted(&widgets::REPO, 14.0, look.accent))
                        .child(
                            text(group.repo.to_string())
                                .font(k.ty.body_medium)
                                .color(look.text)
                                .truncate(),
                        )
                        .child(pill(group.rows.len().to_string(), look.muted, look))
                        .child(row().grow(1.0))
                        .child(
                            text(group.instance.to_string())
                                .font(k.ty.small_medium)
                                .color(look.muted)
                                .truncate(),
                        ),
                ),
            );
            for row_data in group.rows {
                rows = rows.child(item_row(app, row_data, look));
            }
        }
    } else {
        for row_data in &visible {
            rows = rows.child(item_row(app, row_data, look));
        }
    }
    let footer = row()
        .key("list-footer")
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(kbd("j", look))
        .child(kbd("k", look))
        .child(text(app.tr("navegar")).font(k.ty.small).color(look.muted))
        .child(kbd("Enter", look))
        .child(
            text(app.tr("abrir"))
                .font(k.ty.small)
                .color(look.muted)
                .grow(1.0),
        )
        .child(
            Button::new(format!(
                "{}+K {}",
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                },
                app.tr("comandos")
            ))
            .variant(ButtonVariant::Ghost)
            .on_press(|app: &mut App| app.toggle_palette())
            .build(k)
            .key("footer-palette"),
        );
    col()
        .key("list-pane")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(16.0)
        .pt(14.0)
        .gap(10.0)
        .child(heading)
        .children(notifications(app, look))
        .children((!app.sidebar.is_open()).then_some(filters))
        .child(table_head)
        .child(
            rows.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        )
        .child(footer)
}

/// The height of the settings form: what the window leaves around the
/// dialog's title and buttons. The window's size is known from its last frame.
fn settings_height(cx: &mut Cx<App>) -> f32 {
    let window = cx.layout("kit-root").map_or(super::SIZE.1, |rect| rect[3]);
    (window - 180.0).clamp(200.0, 640.0)
}

pub fn view(app: &App, cx: &mut Cx<App>) -> El {
    app.system_dark.set(cx.system_dark());
    let look = theme::look(app.theme, app.palette.as_ref(), cx.system_dark());
    let mut layer = Vec::new();
    let details = inspector::inspector(app, &look, &mut layer);
    let frame = SplitView::new(list_pane(app, &look))
        .sidebar(
            &app.sidebar,
            |app: &mut App| &mut app.sidebar,
            sidebar(app, &look),
        )
        .inspector(&app.inspector, |app: &mut App| &mut app.inspector, details)
        .toolbar(toolbar(app, &look))
        .titlebar_inset(cx.titlebar_inset())
        .reduced_motion(cx.is_reduced_motion())
        .build(&look.kit);
    dialogs::confirm(app, &look, &mut layer);
    dialogs::palette(app, &look, &mut layer);
    dialogs::settings(app, &look, &mut layer, settings_height(cx));
    rune_kit::root(frame.fill(look.background), layer)
}
