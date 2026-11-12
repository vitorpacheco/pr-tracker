//! The modal surfaces: confirmation, the command palette and settings.

use pr_tracker_core::app::Action;
use pr_tracker_core::config::Provider;
use rune::{Cursor, El, FileDialogOptions, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Dialog, Placement, Select, TextArea, TextInput};

use super::run_command;
use super::settings::{
    DIFF_TOOLS, Field, InstanceForm, LANGUAGES, MERGE_METHODS, PROVIDERS, RepoForm, SettingsForm,
    TERMINALS, TOOLS,
};
use super::state::App;
use super::theme::{Look, ThemePreference};
use super::widgets::{BannerKind, activity, banner, eyebrow, kbd, rule};

fn paragraph(message: String, look: &Look) -> El {
    text(message).font(look.kit.ty.ui).color(look.muted).wrap()
}

pub fn confirm(app: &App, look: &Look, layer: &mut Vec<El>) {
    let Some(confirm) = &app.confirm else { return };
    let k = &look.kit;
    let request = &confirm.request;
    let mut body = col().w_full().px(16.0).pb(16.0).gap(10.0).child(
        text(confirm.target.clone())
            .font(k.ty.ui_medium)
            .color(look.text)
            .wrap(),
    );
    if request.force {
        body = body
            .child(text(confirm.worktree.clone()).font(k.ty.mono).color(look.muted).wrap_anywhere())
            .child(banner(
                "confirm-warning",
                app.tr("As alterações não commitadas serão perdidas. Esta operação não pode ser desfeita."),
                BannerKind::Error,
                None,
                look,
            ));
    }
    match request.action {
        Action::UpdateBranch => {
            let how = if confirm.provider == Provider::GitLab {
                "Faz rebase da branch de origem sobre a branch base no servidor e publica a atualização."
            } else {
                "Incorpora os commits da branch base na branch de origem por merge no servidor e publica a atualização."
            };
            body = body
                .child(
                    text(format!("{} → {}", confirm.base, confirm.source))
                        .font(k.ty.mono)
                        .color(look.muted)
                        .wrap_anywhere(),
                )
                .child(paragraph(app.tr(how), look));
        }
        Action::UpdateWorktree => {
            body = body
                .child(paragraph(
                    app.tr("Busca os novos commits da branch de origem do PR/MR e atualiza o worktree local sem criar commits de merge. Alterações locais ou histórico divergente impedem a atualização."),
                    look,
                ))
                .child(
                    Checkbox::new(request.push)
                        .label(app.tr("Fazer push após atualizar a branch"))
                        .on_toggle(|app: &mut App| {
                            if let Some(confirm) = &mut app.confirm {
                                confirm.request.push = !confirm.request.push;
                            }
                        })
                        .build(k)
                        .key("confirm-push"),
                )
                .child(paragraph(app.tr("Envia para a branch de origem no remoto configurado como upstream do worktree, sem force push."), look));
        }
        Action::Comment => {
            body = body.child(
                col()
                    .key("confirm-comment")
                    .w_full()
                    .max_h(220.0)
                    .padding(10.0)
                    .radius(k.radius.md)
                    .fill(look.raised)
                    .child(
                        text(request.body.clone())
                            .font(k.ty.mono)
                            .color(look.text)
                            .wrap()
                            .selectable(true),
                    )
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    }),
            );
        }
        Action::Merge => {
            if let Some(merge) = &confirm.merge {
                let yes_no = |on: bool| app.tr(if on { "sim" } else { "não" });
                body = body.child(
                    col()
                        .key("confirm-merge-options")
                        .gap(2.0)
                        .child(
                            text(format!("{} {}", app.tr("Método:"), merge.method))
                                .font(k.ty.ui)
                                .color(look.text),
                        )
                        .child(
                            text(format!("Auto-merge: {}", yes_no(merge.auto)))
                                .font(k.ty.ui)
                                .color(look.text),
                        )
                        .child(
                            text(format!(
                                "{} {}",
                                app.tr("Apagar branch de origem:"),
                                yes_no(merge.delete_branch)
                            ))
                            .font(k.ty.ui)
                            .color(look.text),
                        ),
                );
            }
        }
        _ => {}
    }
    let danger = request.force || matches!(request.action, Action::Close | Action::RemoveWorktree);
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .pt(6.0)
            .justify_end()
            .child(
                Button::new(app.tr("Cancelar"))
                    .variant(ButtonVariant::Outline)
                    .on_press(|app: &mut App| app.confirm = None)
                    .build(k)
                    .key("confirm-cancel"),
            )
            .child(
                Button::new(app.tr("Confirmar"))
                    .variant(if danger {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Contrast
                    })
                    .on_press(|app: &mut App| app.execute())
                    .build(k)
                    .key("confirm-accept"),
            ),
    );
    Dialog::new("confirm", format!("{}?", app.tr(confirm.label)))
        .open(true)
        .fit()
        .width(480.0)
        .child(body)
        .on_close(|app: &mut App| app.confirm = None)
        .build(k, layer);
}

pub fn palette(app: &App, look: &Look, layer: &mut Vec<El>) {
    if !app.palette_open {
        return;
    }
    let k = &look.kit;
    let commands = app.palette_commands();
    let first = commands.first().map(|command| command.0);
    let search = TextInput::new(&app.palette_query)
        .placeholder(app.tr("Digite uma ação…"))
        .label(app.tr("Buscar comando"))
        .leading_icon(k.glyph.search)
        .width(None)
        .on_edit(|app: &mut App| &mut app.palette_query)
        .on_submit(move |app: &mut App, tx: &mut Tx| {
            if let Some(command) = first {
                run_command(app, command, tx);
            }
        })
        .build(k)
        .key("palette-search");
    let mut list = col().key("palette-commands").w_full().gap(2.0);
    if commands.is_empty() {
        list = list.child(paragraph(app.tr("Nenhum comando encontrado."), look));
    }
    for (index, (command, label, key)) in commands.into_iter().enumerate() {
        list = list.child(
            row()
                .key(format!("palette-{index}"))
                .role("menuitem")
                .label(label.clone())
                .focusable(true)
                .cursor(Cursor::Pointer)
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.md)
                .hover(k.c.hover)
                .on_press_tx(move |app: &mut App, tx: &mut Tx| run_command(app, command, tx))
                .child(text(label).font(k.ty.ui).color(look.text).grow(1.0))
                .child(kbd(key, look)),
        );
    }
    let modifier = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    };
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(10.0)
        .child(search)
        .child(list)
        .child(
            text(format!(
                "{modifier}{}",
                app.tr("+K abre comandos · Esc fecha")
            ))
            .font(k.ty.small)
            .color(look.muted),
        );
    Dialog::new("palette", app.tr("Comandos"))
        .open(true)
        .fit()
        .width(520.0)
        .child(body)
        .on_close(|app: &mut App| app.palette_open = false)
        .build(k, layer);
}

fn edit_of(app: &mut App, field: Field) -> &mut TextEdit {
    let App {
        settings, scratch, ..
    } = app;
    match settings.as_mut().and_then(|form| form.edit(field)) {
        Some(edit) => edit,
        None => scratch,
    }
}

fn labeled(label: String, control: El, look: &Look) -> El {
    col()
        .grow(1.0)
        .min_w(180.0)
        .gap(4.0)
        .child(text(label).font(look.kit.ty.small_medium).color(look.muted))
        .child(control)
}

fn input(label: String, edit: &TextEdit, placeholder: String, field: Field, look: &Look) -> El {
    let control = TextInput::new(edit)
        .label(label.clone())
        .placeholder(placeholder)
        .width(None)
        .on_edit(move |app: &mut App| edit_of(app, field))
        .build(&look.kit)
        .key(format!("field-{field:?}"));
    labeled(label, control, look)
}

/// A select over `(value, label)` options. Labels are catalog keys.
#[allow(clippy::too_many_arguments)]
fn choice(
    app: &App,
    form: &SettingsForm,
    key: String,
    label: String,
    options: &[(String, String)],
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
    look: &Look,
    layer: &mut Vec<El>,
) -> El {
    let labels: Vec<&str> = options.iter().map(|(_, label)| label.as_str()).collect();
    let shown = options
        .iter()
        .find(|(value, _)| value == current)
        .map(|(_, label)| label.as_str());
    let options = options.to_vec();
    let (toggle_key, open) = (
        key.clone(),
        form.open_select.as_deref() == Some(key.as_str()),
    );
    let close = |app: &mut App| {
        if let Some(form) = &mut app.settings {
            form.open_select = None;
        }
    };
    let control = Select::new(key.clone(), &labels)
        .label(label.clone())
        .current(shown)
        .open(open)
        // The trigger stretches across its column, so the list lines up with
        // the value at its start rather than the far end.
        .placement(Placement::BottomStart)
        .on_toggle(move |app: &mut App| {
            if let Some(form) = &mut app.settings {
                form.open_select = if form.open_select.as_deref() == Some(toggle_key.as_str()) {
                    None
                } else {
                    Some(toggle_key.clone())
                };
            }
        })
        .on_close(close)
        .on_choose(move |app: &mut App, picked: &str, _: &mut Tx| {
            if let Some((value, _)) = options.iter().find(|(_, label)| label == picked) {
                choose(app, value);
            }
            close(app);
        })
        // The kit keys the trigger `<key>-trigger` and anchors the listbox to
        // that key, so the trigger must keep it.
        .build(&look.kit, layer);
    let _ = app;
    labeled(label, control, look)
}

fn translated(app: &App, options: &[(&str, &str)]) -> Vec<(String, String)> {
    options
        .iter()
        .map(|(value, label)| (value.to_string(), app.tr(label)))
        .collect()
}

fn fieldset(key: String, title: String, look: &Look) -> El {
    col()
        .key(key)
        .w_full()
        .gap(10.0)
        .padding(12.0)
        .radius(look.kit.radius.md)
        .border(1.0, look.border)
        .child(text(title).font(look.kit.ty.ui_medium).color(look.text))
}

fn grid() -> El {
    row().w_full().gap(10.0).flex_wrap().items_end()
}

fn toggle(
    key: String,
    label: String,
    checked: bool,
    look: &Look,
    flip: impl Fn(&mut SettingsForm) + 'static,
) -> El {
    Checkbox::new(checked)
        .label(label)
        .on_toggle(move |app: &mut App| {
            if let Some(form) = &mut app.settings {
                flip(form);
            }
        })
        .build(&look.kit)
        .key(key)
}

fn danger_link(
    key: String,
    label: String,
    look: &Look,
    remove: impl Fn(&mut SettingsForm) + 'static,
) -> El {
    row().child(
        Button::new(label)
            .variant(ButtonVariant::Ghost)
            .on_press(move |app: &mut App| {
                if let Some(form) = &mut app.settings {
                    remove(form);
                }
            })
            .build(&look.kit)
            .key(key),
    )
}

fn instance_fields(
    app: &App,
    form: &SettingsForm,
    index: usize,
    instance: &InstanceForm,
    look: &Look,
    layer: &mut Vec<El>,
) -> El {
    let k = &look.kit;
    let id = instance.id;
    let providers: Vec<(String, String)> = PROVIDERS
        .iter()
        .map(|provider| {
            let label = match provider {
                Provider::GitHub => "GitHub",
                Provider::GitLab => "GitLab",
                _ => "Gitea",
            };
            (provider.as_str().to_string(), label.to_string())
        })
        .collect();
    let fields = grid()
        .child(input(
            app.tr("Nome"),
            &instance.name,
            String::new(),
            Field::InstanceName(id),
            look,
        ))
        .child(choice(
            app,
            form,
            format!("instance-provider-{id}"),
            "Provider".into(),
            &providers,
            instance.provider.as_str(),
            move |app, value| {
                if let (Some(form), Some(provider)) = (&mut app.settings, Provider::parse(value))
                    && let Some(instance) = form.instance_mut(id)
                {
                    instance.provider = provider;
                }
            },
            look,
            layer,
        ))
        .child(input(
            "Host".into(),
            &instance.host,
            instance.provider.default_host().to_string(),
            Field::InstanceHost(id),
            look,
        ))
        .child(choice(
            app,
            form,
            format!("instance-merge-{id}"),
            app.tr("Método de merge"),
            &translated(app, &MERGE_METHODS),
            &instance.merge_method,
            move |app, value| {
                if let Some(instance) = app.settings.as_mut().and_then(|form| form.instance_mut(id))
                {
                    instance.merge_method = value.to_string();
                }
            },
            look,
            layer,
        ));
    let flip = move |form: &mut SettingsForm, change: fn(&mut InstanceForm)| {
        if let Some(instance) = form.instance_mut(id) {
            change(instance);
        }
    };
    let mut presets = col()
        .key(format!("presets-{id}"))
        .w_full()
        .gap(8.0)
        .child(text(app.tr("Comentários predefinidos")).font(k.ty.small_medium).color(look.text))
        .child(paragraph(app.tr("Botões de comentário nos PRs/MRs desta instância. O envio sempre exige confirmação."), look));
    for (number, preset) in instance.presets.iter().enumerate() {
        let preset_id = preset.id;
        let body = TextArea::new(&preset.body)
            .label(app.tr("Texto do comentário"))
            .rows(3)
            .width(None)
            .on_edit(move |app: &mut App| edit_of(app, Field::PresetBody(id, preset_id)))
            .build(k)
            .key(format!("preset-body-{preset_id}"));
        presets = presets.child(
            fieldset(
                format!("preset-{preset_id}"),
                format!("{} {}", app.tr("Comentário"), number + 1),
                look,
            )
            .child(grid().child(input(
                app.tr("Nome do botão"),
                &preset.label,
                String::new(),
                Field::PresetLabel(id, preset_id),
                look,
            )))
            .child(labeled(app.tr("Texto do comentário"), body, look))
            .child(danger_link(
                format!("preset-remove-{preset_id}"),
                app.tr("Remover comentário"),
                look,
                move |form| form.remove_preset(id, preset_id),
            )),
        );
    }
    presets = presets.child(
        row().child(
            Button::new(app.tr("Adicionar comentário predefinido"))
                .variant(ButtonVariant::Outline)
                .on_press(move |app: &mut App| {
                    if let Some(form) = &mut app.settings {
                        form.add_preset(id);
                    }
                })
                .build(k)
                .key(format!("preset-add-{id}")),
        ),
    );
    fieldset(
        format!("instance-form-{id}"),
        format!("{} {}", app.tr("Instância"), index + 1),
        look,
    )
    .child(fields)
    .child(toggle(
        format!("instance-disabled-{id}"),
        app.tr("Desativada"),
        instance.disabled,
        look,
        move |form| flip(form, |i| i.disabled = !i.disabled),
    ))
    .child(toggle(
        format!("instance-auto-{id}"),
        app.tr("Auto-merge quando o pipeline passar"),
        instance.auto_merge,
        look,
        move |form| flip(form, |i| i.auto_merge = !i.auto_merge),
    ))
    .child(toggle(
        format!("instance-delete-{id}"),
        app.tr("Apagar branch de origem após merge"),
        instance.delete_branch,
        look,
        move |form| flip(form, |i| i.delete_branch = !i.delete_branch),
    ))
    .child(presets)
    .child(danger_link(
        format!("instance-remove-{id}"),
        app.tr("Remover da configuração"),
        look,
        move |form| form.remove_instance(id),
    ))
}

fn repo_fields(
    app: &App,
    form: &SettingsForm,
    repo: &RepoForm,
    look: &Look,
    layer: &mut Vec<El>,
) -> El {
    let k = &look.kit;
    let id = repo.id;
    let instances: Vec<(String, String)> = form
        .instances
        .iter()
        .map(|instance| (instance.id.to_string(), form.instance_label(instance.id)))
        .collect();
    let pick = Button::new(app.tr("Selecionar pasta…"))
        .variant(ButtonVariant::Outline)
        .disabled(app.settings_busy())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            let options = FileDialogOptions {
                title: app.tr("Selecionar clone local"),
                ..FileDialogOptions::default()
            };
            tx.open_folder(options, move |app: &mut App, result, _| match result {
                Ok(Some(path)) => edit_of(app, Field::RepoPath(id))
                    .replace_text(path.to_string_lossy().into_owned()),
                Ok(None) => {}
                Err(err) => app.error = err.to_string(),
            });
        })
        .build(k)
        .key(format!("repo-pick-{id}"));
    fieldset(format!("repo-form-{id}"), repo.name.text.clone(), look)
        .child(
            grid()
                .child(choice(
                    app,
                    form,
                    format!("repo-instance-{id}"),
                    app.tr("Instância"),
                    &instances,
                    &repo.instance.to_string(),
                    move |app, value| {
                        if let (Some(repo), Ok(instance)) = (
                            app.settings.as_mut().and_then(|form| form.repo_mut(id)),
                            value.parse(),
                        ) {
                            repo.instance = instance;
                        }
                    },
                    look,
                    layer,
                ))
                .child(input(
                    app.tr("Repositório"),
                    &repo.name,
                    app.tr("organização/repo"),
                    Field::RepoName(id),
                    look,
                )),
        )
        .child(
            grid()
                .child(input(
                    app.tr("Pasta do clone"),
                    &repo.path,
                    "/home/…".into(),
                    Field::RepoPath(id),
                    look,
                ))
                .child(pick)
                .child(input(
                    "Remote".into(),
                    &repo.remote,
                    "origin".into(),
                    Field::RepoRemote(id),
                    look,
                )),
        )
        .child(toggle(
            format!("repo-track-{id}"),
            app.tr("Acompanhar todos os PRs"),
            repo.track_all,
            look,
            move |form| {
                if let Some(repo) = form.repo_mut(id) {
                    repo.track_all = !repo.track_all;
                }
            },
        ))
        .child(danger_link(
            format!("repo-remove-{id}"),
            app.tr("Remover mapeamento"),
            look,
            move |form| form.remove_repo(id),
        ))
}

fn diagnostics(app: &App, look: &Look) -> El {
    let k = &look.kit;
    let label = if app.pending.contains_key("settings-diagnose") {
        "Verificando…"
    } else {
        "Executar diagnóstico"
    };
    let mut out = col().key("diagnostics").w_full().gap(6.0).child(
        row().child(
            Button::new(app.tr(label))
                .variant(ButtonVariant::Outline)
                .disabled(app.settings_busy())
                .on_press(|app: &mut App| app.diagnose())
                .build(k)
                .key("diagnose"),
        ),
    );
    let Some(found) = &app.diagnostics else {
        return out;
    };
    for tool in &found.tools {
        let (mark, ink) = if tool.path.is_some() {
            ("✓", look.green)
        } else {
            ("×", look.red)
        };
        let place = tool.path.as_ref().map_or_else(
            || {
                format!(
                    "{} · {}",
                    app.tr("Não encontrada no PATH"),
                    app.tr(tool.hint)
                )
            },
            |path| path.display().to_string(),
        );
        out = out.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text(mark).font(k.ty.ui_medium).color(ink).w(14.0))
                .child(
                    text(tool.name)
                        .font(k.ty.ui_medium)
                        .color(look.text)
                        .w(44.0),
                )
                .child(
                    text(place)
                        .font(k.ty.ui)
                        .color(look.muted)
                        .wrap_anywhere()
                        .grow(1.0),
                ),
        );
    }
    for instance in &found.instances {
        let status = if instance.disabled {
            app.tr("desativada")
        } else if instance.error.is_empty() {
            app.tr("autenticada")
        } else {
            instance.error.clone()
        };
        out = out.child(
            text(format!("{}: {status}", instance.name))
                .font(k.ty.ui)
                .color(look.muted)
                .wrap()
                .selectable(true),
        );
    }
    out
}

pub fn settings(app: &App, look: &Look, layer: &mut Vec<El>, height: f32) {
    let Some(form) = &app.settings else { return };
    let k = &look.kit;
    // Selects open above the dialog, so their popups are mounted after it.
    let mut popups = Vec::new();
    let heading = |label: &str| {
        text(app.tr(label))
            .font(k.ty.body_medium)
            .color(look.text)
            .pt(6.0)
    };
    let mut body = col()
        .key("settings-body")
        .w_full()
        .px(16.0)
        .pb(8.0)
        .gap(12.0)
        .child(eyebrow(app.tr("PREFERÊNCIAS"), look));
    for (name, label) in app
        .pending
        .iter()
        .filter(|(name, _)| name.starts_with("settings-"))
    {
        body = body.child(activity(label.clone(), look).key(format!("settings-activity-{name}")));
    }
    if !app.error.is_empty() {
        body = body.child(banner(
            "settings-error",
            app.error.clone(),
            BannerKind::Error,
            Some(|app| app.error.clear()),
            look,
        ));
    }
    let themes: Vec<(String, String)> = ThemePreference::ALL
        .iter()
        .map(|theme| (theme.as_str().to_string(), app.tr(theme.label())))
        .collect();
    let pick_theme = Button::new(app.tr("Selecionar arquivo"))
        .variant(ButtonVariant::Outline)
        .disabled(app.settings_busy())
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            let options = FileDialogOptions {
                title: app.tr("Arquivo de cores"),
                ..FileDialogOptions::default()
            }
            .filter("TOML", &["toml"]);
            tx.open_file(options, |app: &mut App, result, _| match result {
                Ok(Some(path)) => {
                    edit_of(app, Field::ThemeFile).replace_text(path.to_string_lossy().into_owned())
                }
                Ok(None) => {}
                Err(err) => app.error = err.to_string(),
            });
        })
        .build(k)
        .key("theme-pick");
    let export_theme = Button::new(app.tr("Exportar esquema de cores"))
        .variant(ButtonVariant::Outline)
        .disabled(app.settings_busy())
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            let options = FileDialogOptions {
                title: app.tr("Exportar esquema de cores"),
                file_name: Some("colors.toml".into()),
                ..FileDialogOptions::default()
            }
            .filter("TOML", &["toml"]);
            tx.save_file(options, |app: &mut App, result, _| match result {
                Ok(Some(path)) => app.export_theme(path),
                Ok(None) => {}
                Err(err) => app.error = err.to_string(),
            });
        })
        .build(k)
        .key("theme-export");
    body = body
        .child(
            grid()
                .child(choice(
                    app,
                    form,
                    "settings-theme".into(),
                    app.tr("Tema"),
                    &themes,
                    app.theme.as_str(),
                    |app, value| app.set_theme(ThemePreference::parse(value)),
                    look,
                    &mut popups,
                ))
                .child(choice(
                    app,
                    form,
                    "settings-language".into(),
                    app.tr("Idioma"),
                    &translated(app, &LANGUAGES),
                    &form.language,
                    |app, value| {
                        if let Some(form) = &mut app.settings {
                            form.language = value.to_string();
                        }
                    },
                    look,
                    &mut popups,
                )),
        )
        .child(
            grid()
                .child(input(
                    app.tr("Arquivo de cores"),
                    &form.theme_file,
                    "colors.toml".into(),
                    Field::ThemeFile,
                    look,
                ))
                .child(pick_theme)
                .child(export_theme),
        )
        .child(paragraph(
            app.tr("Vazio segue o tema do sistema; caminho relativo à configuração"),
            look,
        ))
        .child(
            grid()
                .child(input(
                    app.tr("Intervalo de atualização"),
                    &form.refresh_interval,
                    "5m".into(),
                    Field::RefreshInterval,
                    look,
                ))
                .child(input(
                    app.tr("Pasta de worktrees"),
                    &form.worktree_dir,
                    app.tr("Padrão da aplicação"),
                    Field::WorktreeDir,
                    look,
                )),
        )
        .child(
            grid()
                .child(choice(
                    app,
                    form,
                    "settings-terminal".into(),
                    "Terminal".into(),
                    &translated(app, &TERMINALS),
                    &form.terminal,
                    |app, value| {
                        if let Some(form) = &mut app.settings {
                            form.terminal = value.to_string();
                        }
                    },
                    look,
                    &mut popups,
                ))
                .child(input(
                    app.tr("Terminal desktop"),
                    &form.desktop_terminal,
                    app.tr("Detectar automaticamente (kitty, foot, wezterm…)"),
                    Field::DesktopTerminal,
                    look,
                ))
                .child(choice(
                    app,
                    form,
                    "settings-diff".into(),
                    "Diff".into(),
                    &translated(app, &DIFF_TOOLS),
                    &form.diff_tool,
                    |app, value| {
                        if let Some(form) = &mut app.settings {
                            form.diff_tool = value.to_string();
                        }
                    },
                    look,
                    &mut popups,
                )),
        )
        .child(heading("Caminhos das ferramentas"));
    let mut tools = grid();
    for (index, tool) in TOOLS.iter().enumerate() {
        tools = tools.child(input(
            tool.to_string(),
            &form.tools[index],
            app.tr("Detectar no PATH"),
            Field::Tool(index),
            look,
        ));
    }
    body = body.child(tools).child(heading("Instâncias"));
    for (index, instance) in form.instances.iter().enumerate() {
        body = body.child(instance_fields(
            app,
            form,
            index,
            instance,
            look,
            &mut popups,
        ));
    }
    body = body
        .child(
            row().child(
                Button::new(app.tr("＋ Adicionar instância"))
                    .variant(ButtonVariant::Outline)
                    .on_press(|app: &mut App| {
                        if let Some(form) = &mut app.settings {
                            form.add_instance();
                        }
                    })
                    .build(k)
                    .key("instance-form-add"),
            ),
        )
        .child(heading("Repositórios locais"));
    for repo in &form.repos {
        body = body.child(repo_fields(app, form, repo, look, &mut popups));
    }
    body = body
        .child(
            row().child(
                Button::new(app.tr("＋ Mapear repositório"))
                    .variant(ButtonVariant::Outline)
                    .on_press(|app: &mut App| {
                        if let Some(form) = &mut app.settings {
                            form.add_repo();
                        }
                    })
                    .build(k)
                    .key("repo-form-add"),
            ),
        )
        .child(heading("Ferramentas e autenticação"))
        .child(diagnostics(app, look));
    let saving = app.pending.contains_key("settings-save");
    let footer = row()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .justify_end()
        .child(
            Button::new(app.tr("Cancelar"))
                .variant(ButtonVariant::Outline)
                .on_press(|app: &mut App| app.settings = None)
                .build(k)
                .key("settings-cancel"),
        )
        .child(
            Button::new(app.tr(if saving {
                "Salvando…"
            } else {
                "Salvar configurações"
            }))
            .variant(ButtonVariant::Contrast)
            .disabled(app.settings_busy())
            .on_press(|app: &mut App| app.save_settings())
            .build(k)
            .key("settings-save"),
        );
    let content = col()
        .w_full()
        .child(
            body.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .h(height),
        )
        .child(rule(look))
        .child(footer);
    Dialog::new("settings", app.tr("Configurações"))
        .open(true)
        .fit()
        .width(760.0)
        .click_outside(false)
        .child(content)
        .on_close(|app: &mut App| app.settings = None)
        .build(k, layer);
    layer.append(&mut popups);
}
