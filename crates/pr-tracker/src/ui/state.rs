//! What the window knows and how it changes: background work comes back as
//! events, and handlers call the methods here. Nothing in this module draws.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use chrono::{DateTime, Utc};
use pr_tracker_core::app::{self, Action, Application, Command, Diagnostics, ERR_CLONE_REQUIRED, ERR_DIRTY_WORKTREE, Session, SnapshotStore};
use pr_tracker_core::config::{Config, Instance, Provider};
use pr_tracker_core::provider::{Kind, MergeOptions, Relation, Thread};
use pr_tracker_core::theme::{self, Palette};
use pr_tracker_core::toolchain::Ctx;
use pr_tracker_core::{Error, cache, i18n, launch, msg};
use rune::{TextEdit, Waker};
use rune_kit::PaneState;

use super::list::{self, Row};
use super::prefs::Prefs;
use super::settings::SettingsForm;
use super::text::t;
use super::theme::ThemePreference;
use super::{demo, theme as look};
use crate::cli;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Details,
    Checks,
    Conversation,
}

/// What the user asked to do to an item, awaiting or past confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub key: String,
    pub action: Action,
    pub body: String,
    pub remove_after: bool,
    pub force: bool,
    pub push: bool,
}

pub struct Confirm {
    /// The question's subject; a catalog key.
    pub label: &'static str,
    pub request: Request,
    pub target: String,
    pub worktree: String,
    pub source: String,
    pub base: String,
    pub provider: Provider,
    /// The body came from the comment box, which empties once sent.
    pub uses_draft: bool,
    pub merge: Option<MergeOptions>,
}

/// The list state an event brings back, with the configuration it was read under.
pub struct Fresh {
    state: app::State,
    cfg: Config,
}

pub enum Event {
    Loaded(Result<Fresh, Error>),
    Refreshed(Result<Fresh, Error>),
    Detail { generation: u64, result: Result<Thread, Error> },
    Executed { request: Request, uses_draft: bool, message: String, error: Option<Error>, reload_thread: bool, fresh: Result<Fresh, Error> },
    SettingsSaved { result: Result<Fresh, Error>, notice: &'static str },
    Diagnosed(Result<Diagnostics, Error>),
    Palette(Result<Option<Palette>, Error>),
    ThemeExported(Result<PathBuf, Error>),
}

/// An entry of the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    Refresh,
    Search,
    Conversation,
    Comment,
    Browser,
    Worktree,
    Terminal,
    Diff,
    Approve,
    Merge,
    Settings,
}

impl PaletteCommand {
    pub const ALL: [(PaletteCommand, &'static str, &'static str); 11] = [
        (PaletteCommand::Refresh, "Atualizar agora", "r"),
        (PaletteCommand::Search, "Buscar itens", "/"),
        (PaletteCommand::Conversation, "Ver conversa", "v"),
        (PaletteCommand::Comment, "Comentar", "n"),
        (PaletteCommand::Browser, "Abrir no navegador", "o"),
        (PaletteCommand::Worktree, "Criar / atualizar worktree", "w"),
        (PaletteCommand::Terminal, "Abrir terminal", "t"),
        (PaletteCommand::Diff, "Abrir diff", "d"),
        (PaletteCommand::Approve, "Aprovar", "a"),
        (PaletteCommand::Merge, "Merge", "m"),
        (PaletteCommand::Settings, "Configurações", ","),
    ];
}

pub struct App {
    session: Arc<Session>,
    sender: Sender<Event>,
    events: Receiver<Event>,
    waker: Option<Waker>,
    /// Fictional data: nothing is read from or written to the user's files.
    pub demo: bool,
    /// The configuration could not be read, so saving would overwrite it.
    settings_blocked: Option<String>,
    seed_on_start: bool,

    pub cfg: Config,
    pub language: &'static str,
    pub rows: Vec<Row>,
    pub errors: BTreeMap<String, String>,
    pub cache_error: String,
    pub synced_at: Option<DateTime<Utc>>,
    revision: u64,

    pub kind: Kind,
    pub relation: Relation,
    pub instance: String,
    pub query: TextEdit,
    pub selected: String,
    pub detail_tab: DetailTab,

    pub loading: bool,
    pub refreshing: bool,
    pub error: String,
    pub theme_error: String,
    pub notice: String,
    /// Work in progress on an item, by item key.
    pub busy: BTreeMap<String, String>,
    /// Other work in progress, by a fixed name.
    pub pending: BTreeMap<&'static str, String>,

    pub thread: Option<Thread>,
    pub detail_error: String,
    pub detail_loading: bool,
    detail_generation: u64,

    pub draft: TextEdit,
    drafts: BTreeMap<String, String>,

    pub confirm: Option<Confirm>,
    pub palette_open: bool,
    pub palette_query: TextEdit,
    pub more_open: bool,
    pub settings: Option<SettingsForm>,
    pub diagnostics: Option<Diagnostics>,
    /// Stands in for a settings field that is gone by the time its handler runs.
    pub scratch: TextEdit,

    pub theme: ThemePreference,
    pub palette: Option<Palette>,
    palette_pending: bool,
    pub sidebar: PaneState,
    pub inspector: PaneState,
    saved: Prefs,

    /// Whether the system appearance was dark when the window last drew.
    pub system_dark: std::cell::Cell<bool>,

    /// Seconds on the window's clock; zero means not yet scheduled.
    refresh_due: f64,
    housekeeping_due: f64,
    /// The clock time the window was last asked to wake at.
    pub wake_at: f64,
}

pub fn sidebar_pane(prefs: &Prefs) -> PaneState {
    let pane = PaneState::new(prefs.sidebar_width).range(180.0, 360.0);
    if prefs.sidebar_collapsed { pane.collapsed() } else { pane }
}

pub fn inspector_pane(prefs: &Prefs) -> PaneState {
    let pane = PaneState::new(prefs.inspector_width).range(300.0, 720.0);
    if prefs.inspector_collapsed { pane.collapsed() } else { pane }
}

impl App {
    /// The window on fictional data.
    pub fn demo() -> App {
        let session = Session::with_application(demo::application());
        let mut app = App::with_session(session, demo::config(), Prefs::default());
        app.demo = true;
        app
    }

    /// The window on the user's configuration and cache.
    pub fn open() -> App {
        let mut startup_error = String::new();
        let mut blocked = None;
        let (cfg, created) = Config::load().unwrap_or_else(|err| {
            let text = err.text(i18n::resolve("system"));
            startup_error = text.clone();
            blocked = Some(text);
            (Config::default(), false)
        });
        let language = i18n::resolve(&cfg.language);
        let store = match cache::Store::open_default() {
            Ok(store) => Some(Arc::new(store) as Arc<dyn SnapshotStore>),
            Err(err) => {
                startup_error = err.text(language);
                None
            }
        };
        let mut app = App::with_session(Session::new(&cfg, store), cfg, Prefs::load());
        app.cache_error = startup_error;
        app.settings_blocked = blocked;
        app.seed_on_start = created;
        app
    }

    fn with_session(session: Arc<Session>, cfg: Config, prefs: Prefs) -> App {
        let (sender, events) = channel();
        App {
            session,
            sender,
            events,
            waker: None,
            demo: false,
            settings_blocked: None,
            seed_on_start: false,
            language: i18n::resolve(&cfg.language),
            cfg,
            rows: Vec::new(),
            errors: BTreeMap::new(),
            cache_error: String::new(),
            synced_at: None,
            revision: 0,
            kind: Kind::Pr,
            relation: Relation::NONE,
            instance: String::new(),
            query: TextEdit::new(""),
            selected: String::new(),
            detail_tab: DetailTab::Details,
            loading: true,
            refreshing: false,
            error: String::new(),
            theme_error: String::new(),
            notice: String::new(),
            busy: BTreeMap::new(),
            pending: BTreeMap::new(),
            thread: None,
            detail_error: String::new(),
            detail_loading: false,
            detail_generation: 0,
            draft: TextEdit::new(""),
            drafts: prefs.drafts.clone(),
            confirm: None,
            palette_open: false,
            palette_query: TextEdit::new(""),
            more_open: false,
            settings: None,
            diagnostics: None,
            scratch: TextEdit::new(""),
            theme: ThemePreference::parse(&prefs.theme),
            palette: None,
            palette_pending: false,
            sidebar: sidebar_pane(&prefs),
            inspector: inspector_pane(&prefs),
            saved: prefs,
            system_dark: std::cell::Cell::new(true),
            refresh_due: 0.0,
            housekeeping_due: 0.0,
            wake_at: 0.0,
        }
    }

    pub fn started(&self) -> bool {
        self.waker.is_some()
    }

    /// Blocks until the background work in flight has been applied.
    pub fn settle(&mut self) {
        let started = std::time::Instant::now();
        let mut quiet = 0;
        while quiet < 5 && started.elapsed() < std::time::Duration::from_secs(10) {
            let idle = !self.tick(1.0) && !self.loading && !self.refreshing && !self.detail_loading && self.busy.is_empty() && self.pending.is_empty();
            quiet = if idle { quiet + 1 } else { 0 };
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn tr(&self, message: &str) -> String {
        t(self.language, message)
    }

    /// Starts reading the cache and the palette. `waker` redraws the window
    /// when background work reports back.
    pub fn begin(&mut self, waker: Waker) {
        self.waker = Some(waker);
        self.poll_palette();
        let seed = std::mem::take(&mut self.seed_on_start);
        self.spawn(move |session| {
            if seed {
                // First run: adopt the hosts the installed CLIs are logged in to.
                let mut cfg = session.configuration();
                cli::seed(&mut cfg);
                let _ = session.save_settings(&cfg);
            }
            Event::Loaded(session.load(&Ctx::background()).map(|state| Fresh { state, cfg: session.configuration() }))
        });
    }

    fn spawn(&self, work: impl FnOnce(&Arc<Session>) -> Event + Send + 'static) {
        let (session, sender, waker) = (self.session.clone(), self.sender.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let event = work(&session);
            // The window may have closed while the work ran.
            let _ = sender.send(event);
            if let Some(wake) = waker {
                wake();
            }
        });
    }

    /// Applies finished background work and runs what is due at `clock`
    /// seconds. Returns whether anything changed.
    pub fn tick(&mut self, clock: f64) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            self.apply(event, clock);
            changed = true;
        }
        if self.refresh_due > 0.0 && clock >= self.refresh_due && !self.loading && !self.refreshing {
            self.refresh();
            changed = true;
        }
        if clock >= self.housekeeping_due {
            self.housekeeping_due = clock + 2.0;
            self.poll_palette();
            self.save_prefs();
        }
        changed
    }

    /// Seconds until [`App::tick`] next has something due.
    pub fn next_wake(&self, clock: f64) -> f64 {
        let mut due = self.housekeeping_due;
        if self.refresh_due > 0.0 {
            due = due.min(self.refresh_due);
        }
        (due - clock).max(0.05)
    }

    fn apply(&mut self, event: Event, clock: f64) {
        match event {
            Event::Loaded(result) => {
                self.loading = false;
                match result {
                    Ok(fresh) => self.set_view(fresh),
                    Err(err) => self.error = err.text(self.language),
                }
                self.refresh();
            }
            Event::Refreshed(result) => {
                self.refreshing = false;
                match result {
                    Ok(fresh) => self.set_view(fresh),
                    Err(err) => self.error = err.text(self.language),
                }
                self.refresh_due = clock + self.cfg.interval().as_secs_f64();
            }
            Event::Detail { generation, result } => {
                if generation != self.detail_generation {
                    return;
                }
                self.detail_loading = false;
                match result {
                    Ok(thread) => self.thread = Some(thread),
                    Err(err) => self.detail_error = err.text(self.language),
                }
            }
            Event::Executed { request, uses_draft, message, error, reload_thread, fresh } => {
                self.busy.remove(&request.key);
                let target = self.row(&request.key).cloned();
                let load_error = match fresh {
                    Ok(fresh) => {
                        self.set_view(fresh);
                        None
                    }
                    Err(err) => Some(err),
                };
                self.notice = message;
                self.error = error.as_ref().or(load_error.as_ref()).map(|err| err.text(self.language)).unwrap_or_default();
                if reload_thread && self.selected == request.key {
                    if uses_draft {
                        self.draft.clear();
                        self.drafts.remove(&request.key);
                    }
                    self.select(&request.key);
                }
                let dirty = error.as_ref().is_some_and(|err| err.is(ERR_DIRTY_WORKTREE));
                if dirty && (request.action == Action::RemoveWorktree || request.remove_after) {
                    self.confirm = Some(Confirm {
                        label: "Descartar alterações locais e remover worktree",
                        target: target.as_ref().map_or_else(|| request.key.clone(), |row| format!("{} {}", row.item.repo, row.reference)),
                        worktree: target.as_ref().and_then(|row| row.worktree.as_deref()).map(display).unwrap_or_default(),
                        source: String::new(),
                        base: String::new(),
                        provider: target.as_ref().map_or(Provider::GitHub, |row| row.item.provider),
                        uses_draft: false,
                        merge: None,
                        request: Request { key: request.key, action: Action::RemoveWorktree, body: String::new(), remove_after: false, force: true, push: false },
                    });
                }
                if error.as_ref().is_some_and(|err| err.is(ERR_CLONE_REQUIRED)) {
                    self.open_settings();
                    self.notice = self.tr("Defina a pasta local do repositório nas configurações e repita a ação.");
                }
            }
            Event::SettingsSaved { result, notice } => {
                self.pending.remove("settings-save");
                match result {
                    Ok(fresh) => {
                        self.set_view(fresh);
                        self.settings = None;
                        self.notice = self.tr(notice);
                        self.refresh_due = clock + self.cfg.interval().as_secs_f64();
                        self.poll_palette();
                    }
                    Err(err) => self.error = err.text(self.language),
                }
            }
            Event::Diagnosed(result) => {
                self.pending.remove("settings-diagnose");
                match result {
                    Ok(diagnostics) => self.diagnostics = Some(diagnostics),
                    Err(err) => self.error = err.text(self.language),
                }
            }
            Event::Palette(result) => {
                self.palette_pending = false;
                match result {
                    Ok(palette) => {
                        self.palette = palette;
                        self.theme_error.clear();
                    }
                    // Keep the last working palette if its file becomes unreadable.
                    Err(err) => self.theme_error = err.text(self.language),
                }
            }
            Event::ThemeExported(result) => {
                self.pending.remove("settings-export");
                match result {
                    Ok(path) => self.notice = format!("{}: {}", self.tr("Esquema de cores exportado"), path.display()),
                    Err(err) => self.error = err.text(self.language),
                }
            }
        }
    }

    fn set_view(&mut self, fresh: Fresh) {
        let Fresh { state, cfg } = fresh;
        if state.revision < self.revision {
            return;
        }
        self.revision = state.revision;
        self.language = i18n::resolve(&cfg.language);
        self.cfg = cfg;
        self.rows = state
            .items
            .into_iter()
            .map(|item| {
                let key = item.key();
                Row { reference: item.reference(), clone: state.clones.get(&key).cloned(), worktree: state.worktrees.get(&key).cloned(), key, item }
            })
            .collect();
        self.errors = state.errors.iter().map(|(name, err)| (name.clone(), err.text(self.language))).collect();
        // A cache that failed to open at startup stays reported until it works.
        if let Some(err) = &state.cache_error {
            self.cache_error = err.text(self.language);
        }
        self.synced_at = state.synced_at;
        if !self.instance.is_empty() && !self.instances().any(|instance| instance.name == self.instance) {
            self.instance.clear();
        }
        self.reselect();
    }

    /// The enabled instances, in configuration order.
    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.cfg.instances.iter().filter(|instance| !instance.disabled)
    }

    pub fn row(&self, key: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.key == key)
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.row(&self.selected)
    }

    /// The rows the list shows, most recently updated first.
    pub fn visible(&self) -> Vec<&Row> {
        list::filter(&self.rows, self.kind, self.relation, &self.query.text, &self.instance)
    }

    pub fn count(&self, kind: Kind, relation: Relation) -> usize {
        list::filter(&self.rows, kind, relation, "", &self.instance).len()
    }

    /// The comment presets offered for the selected item.
    pub fn comment_presets(&self) -> &[pr_tracker_core::config::CommentPreset] {
        self.selected_row()
            .filter(|row| !row.item.is_issue())
            .and_then(|row| self.cfg.instance(&row.item.instance))
            .map_or(&[], |instance| &instance.comment_presets)
    }

    /// Keeps the selection on a row that is showing.
    pub fn reselect(&mut self) {
        let visible = self.visible();
        if visible.iter().any(|row| row.key == self.selected) {
            return;
        }
        match visible.first().map(|row| row.key.clone()) {
            Some(first) => self.select(&first),
            None if !self.selected.is_empty() => {
                self.keep_draft();
                self.selected.clear();
                self.thread = None;
                self.detail_generation += 1;
                self.detail_loading = false;
            }
            None => {}
        }
    }

    fn keep_draft(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        if self.draft.text.is_empty() {
            self.drafts.remove(&self.selected);
        } else {
            self.drafts.insert(self.selected.clone(), self.draft.text.clone());
        }
    }

    /// Shows an item and loads its conversation.
    pub fn select(&mut self, key: &str) {
        self.keep_draft();
        self.selected = key.to_string();
        self.draft = TextEdit::new(self.drafts.get(key).cloned().unwrap_or_default());
        self.thread = None;
        self.detail_error.clear();
        self.detail_loading = true;
        self.more_open = false;
        self.detail_generation += 1;
        let (generation, key) = (self.detail_generation, key.to_string());
        self.spawn(move |session| Event::Detail { generation, result: session.detail(&Ctx::background(), &key) });
    }

    /// Moves the selection by `delta` rows and returns the newly selected key.
    pub fn move_selection(&mut self, delta: isize) -> Option<String> {
        let next = {
            let visible = self.visible();
            let keys: Vec<&str> = visible.iter().map(|row| row.key.as_str()).collect();
            list::next_selection(&keys, &self.selected, delta).map(str::to_string)
        }?;
        if next != self.selected {
            self.select(&next);
        }
        Some(next)
    }

    pub fn switch_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.relation = Relation::NONE;
        self.reselect();
    }

    pub fn set_relation(&mut self, relation: Relation) {
        self.relation = relation;
        self.reselect();
    }

    pub fn set_instance(&mut self, name: &str) {
        self.instance = if self.instance == name { String::new() } else { name.to_string() };
        self.reselect();
    }

    pub fn clear_filters(&mut self) {
        self.query.clear();
        self.relation = Relation::NONE;
        self.instance.clear();
        self.reselect();
    }

    pub fn refresh(&mut self) {
        if self.refreshing || self.loading {
            return;
        }
        self.refreshing = true;
        self.error.clear();
        self.refresh_due = 0.0;
        self.spawn(|session| Event::Refreshed(session.refresh(&Ctx::background()).map(|state| Fresh { state, cfg: session.configuration() })));
    }

    /// Asks for confirmation before acting on the selected item. `body`
    /// overrides the comment box as the text to send.
    pub fn ask(&mut self, action: Action, label: &'static str, force: bool, remove_after: bool, body: Option<String>) {
        let Some(row) = self.selected_row() else { return };
        if self.busy.contains_key(&row.key) {
            return;
        }
        if row.item.is_issue() && action != Action::Comment {
            self.notice = self.tr("Ação disponível apenas para pull/merge requests.");
            return;
        }
        let item = &row.item;
        self.confirm = Some(Confirm {
            label,
            target: format!("{} {}", item.repo, row.reference),
            worktree: row.worktree.as_deref().map(display).unwrap_or_default(),
            source: item.source_branch.clone(),
            base: item.target_branch.clone(),
            provider: item.provider,
            uses_draft: body.is_none(),
            merge: (action == Action::Merge).then(|| Application::new(&self.cfg, None).merge_options(item)),
            request: Request {
                key: row.key.clone(),
                action,
                body: body.unwrap_or_else(|| self.draft.text.clone()),
                remove_after,
                force,
                push: false,
            },
        });
        self.more_open = false;
        self.palette_open = false;
    }

    /// The label of an action in progress; a catalog key.
    fn progress_label(request: &Request) -> &'static str {
        match request.action {
            Action::Comment => "Enviando comentário…",
            Action::Approve => "Aprovando PR/MR…",
            Action::Merge => "Fazendo merge…",
            Action::Close => "Fechando PR/MR…",
            Action::MarkReady => "Marcando como Ready…",
            Action::MarkDraft => "Marcando como Draft…",
            Action::Checkout => "Trocando branch…",
            Action::UpdateWorktree if request.push => "Atualizando branch e fazendo push…",
            Action::UpdateWorktree => "Atualizando worktree…",
            Action::UpdateBranch => "Atualizando branch com a base no servidor…",
            Action::PrepareTerminal => "Preparando terminal…",
            Action::PrepareDiff => "Preparando diff…",
            Action::RemoveWorktree => "Removendo worktree…",
        }
    }

    /// Runs the confirmed action in the background.
    pub fn execute(&mut self) {
        let Some(Confirm { request, uses_draft, target, .. }) = self.confirm.take() else { return };
        self.busy.insert(request.key.clone(), format!("{target} · {}", self.tr(Self::progress_label(&request))));
        self.error.clear();
        self.notice.clear();
        let language = self.language;
        self.spawn(move |session| {
            let ctx = Ctx::background();
            let command = Command { body: request.body.clone(), remove_after: request.remove_after, force: request.force, push: request.push, ..Command::new(request.action) };
            let out = session.execute(&ctx, &request.key, command);
            let (mut message, mut error) = (out.message, out.error);
            if error.is_none() && matches!(request.action, Action::PrepareTerminal | Action::PrepareDiff) {
                let cfg = session.configuration();
                let dir = out.directory.unwrap_or_default();
                let label = request.key.rsplit('|').next().unwrap_or("pr-tracker").to_string();
                let work = ctx.with_paths(&cfg.tool_paths);
                match launch::open(&work, launch::resolve(&cfg.terminal), &cfg.desktop_terminal, &dir, &label, &out.args) {
                    Ok(()) => {
                        message = t(language, "Terminal aberto");
                        if !out.note.is_empty() {
                            message = format!("{message} · {}", out.note);
                        }
                    }
                    Err(err) => error = Some(err),
                }
            }
            let state = if out.refresh { session.refresh(&ctx) } else { session.load(&ctx) };
            Event::Executed {
                uses_draft,
                message,
                error,
                reload_thread: out.reload_thread,
                fresh: state.map(|state| Fresh { state, cfg: session.configuration() }),
                request,
            }
        });
    }

    pub fn open_settings(&mut self) {
        if self.pending.contains_key("settings-save") {
            return;
        }
        self.settings = Some(SettingsForm::new(&self.cfg));
        self.diagnostics = None;
        self.palette_open = false;
        self.more_open = false;
    }

    pub fn settings_busy(&self) -> bool {
        self.pending.keys().any(|key| key.starts_with("settings-"))
    }

    pub fn save_settings(&mut self) {
        let Some(cfg) = self.settings.as_ref().map(SettingsForm::to_config) else { return };
        self.save_config(cfg, "Configurações salvas.");
    }

    /// Persists `cfg` and refreshes under it.
    fn save_config(&mut self, cfg: Config, notice: &'static str) {
        if self.settings_busy() {
            return;
        }
        if self.demo {
            self.error = msg!("Demonstração: esta ação não altera dados.").text(self.language);
            return;
        }
        if let Some(reason) = &self.settings_blocked {
            self.error = reason.clone();
            return;
        }
        self.error.clear();
        self.pending.insert("settings-save", self.tr("Salvando configurações e sincronizando…"));
        self.spawn(move |session| {
            let result = session
                .save_settings(&cfg)
                .and_then(|()| session.refresh(&Ctx::background()))
                .map(|state| Fresh { state, cfg: session.configuration() });
            Event::SettingsSaved { result, notice }
        });
    }

    /// Whether every open pull/merge request of the selected item's
    /// repository is tracked.
    pub fn tracks_all(&self) -> bool {
        self.selected_row().and_then(|row| self.cfg.repo(&row.item.instance, &row.item.repo)).is_some_and(|repo| repo.track_all)
    }

    pub fn toggle_track_all(&mut self) {
        let Some(row) = self.selected_row() else { return };
        let mut cfg = self.cfg.clone();
        cfg.set_repo_track_all(&row.item.instance, &row.item.repo, !self.tracks_all());
        self.more_open = false;
        self.save_config(cfg, "Configurações salvas.");
    }

    /// Maps the selected item's repository to a local clone.
    pub fn set_repo_folder(&mut self, key: &str, path: &Path) {
        let Some(row) = self.row(key) else { return };
        let mut cfg = self.cfg.clone();
        cfg.set_repo_path(&row.item.instance, &row.item.repo, &path.to_string_lossy());
        self.save_config(cfg, "Configurações salvas.");
    }

    pub fn diagnose(&mut self) {
        if self.settings_busy() {
            return;
        }
        self.pending.insert("settings-diagnose", self.tr("Verificando ferramentas e autenticação…"));
        self.spawn(|session| Event::Diagnosed(session.diagnose(&Ctx::background())));
    }

    pub fn set_theme(&mut self, theme: ThemePreference) {
        self.theme = theme;
        self.save_prefs();
    }

    /// Writes the colours on screen to `path` as a reusable palette file.
    pub fn export_theme(&mut self, path: PathBuf) {
        let palette = look::displayed(self.theme, self.palette.as_ref(), self.system_dark.get());
        self.pending.insert("settings-export", self.tr("Exportando esquema de cores…"));
        self.spawn(move |_| Event::ThemeExported(theme::export(&path, &palette).map(|()| path)));
    }

    /// Re-reads the palette so a theme changed outside the window shows up.
    fn poll_palette(&mut self) {
        if self.palette_pending || self.waker.is_none() {
            return;
        }
        self.palette_pending = true;
        let path = if self.demo { None } else { self.cfg.theme_path() };
        let system = !self.demo;
        self.spawn(move |_| {
            Event::Palette(match path {
                Some(path) => theme::resolve(Some(&path)),
                None if system => theme::resolve(None),
                None => Ok(None),
            })
        });
    }

    /// The link of the selected item, when it is safe to hand to a browser.
    pub fn browser_url(&mut self) -> Option<String> {
        let url = self.selected_row()?.item.url.clone();
        let web = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
        if !web {
            self.error = self.tr("link inválido");
            return None;
        }
        Some(url)
    }

    /// Opens the selected item's worktree, or else its clone, in the file manager.
    pub fn open_folder(&mut self) {
        let Some(row) = self.selected_row() else { return };
        let Some(dir) = row.worktree.clone().or_else(|| row.clone.clone()) else {
            self.error = msg!(ERR_CLONE_REQUIRED).text(self.language);
            return;
        };
        self.more_open = false;
        if !dir.is_dir() {
            self.error = self.tr("pasta não encontrada");
            return;
        }
        if let Err(err) = launch::folder(&dir) {
            self.error = err.text(self.language);
        }
    }

    pub fn show_conversation(&mut self, compose: bool) {
        self.detail_tab = DetailTab::Conversation;
        if !self.inspector.is_open() {
            self.inspector.toggle();
        }
        if compose {
            self.draft.focused = true;
        }
    }

    pub fn toggle_palette(&mut self) {
        if self.confirm.is_some() || self.settings.is_some() {
            return;
        }
        self.palette_open = !self.palette_open;
        self.palette_query = TextEdit::new("");
        self.palette_query.focused = self.palette_open;
    }

    /// The palette commands matching what was typed.
    pub fn palette_commands(&self) -> Vec<(PaletteCommand, String, &'static str)> {
        let query = self.palette_query.text.to_lowercase();
        PaletteCommand::ALL
            .iter()
            .map(|&(command, label, key)| (command, self.tr(label), key))
            .filter(|(_, label, _)| label.to_lowercase().contains(&query))
            .collect()
    }

    /// Whether a dialog owns the keyboard.
    pub fn modal(&self) -> bool {
        self.confirm.is_some() || self.settings.is_some() || self.palette_open
    }

    fn prefs(&self) -> Prefs {
        let mut drafts = self.drafts.clone();
        if !self.selected.is_empty() {
            if self.draft.text.is_empty() {
                drafts.remove(&self.selected);
            } else {
                drafts.insert(self.selected.clone(), self.draft.text.clone());
            }
        }
        Prefs {
            theme: self.theme.as_str().to_string(),
            sidebar_width: self.sidebar.width(),
            sidebar_collapsed: !self.sidebar.is_open(),
            inspector_width: self.inspector.width(),
            inspector_collapsed: !self.inspector.is_open(),
            drafts,
        }
    }

    /// Writes the window preferences when they changed.
    pub fn save_prefs(&mut self) {
        if self.demo {
            return;
        }
        let prefs = self.prefs();
        if prefs != self.saved {
            prefs.save();
            self.saved = prefs;
        }
    }

    /// Stops background work; called as the window closes.
    pub fn close(&mut self) {
        self.save_prefs();
        self.session.close();
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Runs `app` until background work settles.
    pub(crate) fn settle(app: &mut App) {
        app.settle();
        assert!(!app.loading && !app.refreshing && app.busy.is_empty() && app.pending.is_empty(), "background work did not settle");
    }

    pub(crate) fn demo() -> App {
        let mut app = App::demo();
        app.begin(Arc::new(|| {}));
        settle(&mut app);
        app
    }

    #[test]
    fn startup_loads_refreshes_and_selects_the_first_row() {
        let app = demo();
        assert_eq!(app.rows.len(), 9);
        assert_eq!(app.visible().len(), 8, "pull requests only");
        assert_eq!(app.selected, app.visible()[0].key);
        assert!(app.thread.as_ref().is_some_and(|thread| thread.body.contains("Bearer token")));
        assert!(app.synced_at.is_some() && app.error.is_empty());
        assert_eq!(app.instances().count(), 2);
    }

    #[test]
    fn filters_narrow_the_list_and_move_the_selection() {
        let mut app = demo();
        app.switch_kind(Kind::Issue);
        settle(&mut app);
        assert_eq!(app.visible().len(), 1);
        assert!(app.selected_row().is_some_and(|row| row.item.is_issue()));
        app.switch_kind(Kind::Pr);
        app.set_relation(Relation::AUTHORED);
        assert_eq!(app.visible().len(), 2);
        app.query.replace_text("docs");
        app.reselect();
        assert_eq!(app.visible().len(), 1);
        app.query.replace_text("nothing matches");
        app.reselect();
        assert!(app.selected.is_empty() && app.thread.is_none());
        app.clear_filters();
        assert_eq!(app.visible().len(), 8);
        assert!(!app.selected.is_empty());
        app.set_instance("Corp Git");
        assert_eq!(app.count(Kind::Pr, Relation::NONE), 5);
        app.set_instance("Corp Git");
        assert!(app.instance.is_empty(), "choosing the instance again clears it");
    }

    #[test]
    fn arrow_keys_walk_the_visible_rows() {
        let mut app = demo();
        let keys: Vec<String> = app.visible().iter().map(|row| row.key.clone()).collect();
        assert_eq!(app.move_selection(1), Some(keys[1].clone()));
        assert_eq!(app.move_selection(-1), Some(keys[0].clone()));
        assert_eq!(app.move_selection(-1), Some(keys[0].clone()), "stays on the first row");
    }

    #[test]
    fn actions_need_confirmation_and_report_their_outcome() {
        let mut app = demo();
        app.ask(Action::Merge, "Fazer merge", false, false, None);
        let confirm = app.confirm.as_ref().expect("merge asks first");
        assert_eq!(confirm.merge.as_ref().map(|merge| merge.method.as_str()), Some("squash"));
        assert!(app.busy.is_empty(), "nothing runs before confirmation");
        app.execute();
        assert_eq!(app.busy.len(), 1);
        settle(&mut app);
        assert!(app.confirm.is_none() && app.busy.is_empty());
        assert_eq!(app.error, t(app.language, "Demonstração: esta ação não altera dados."));
    }

    #[test]
    fn issues_only_accept_comments() {
        let mut app = demo();
        app.switch_kind(Kind::Issue);
        settle(&mut app);
        app.ask(Action::Approve, "Aprovar pull request", false, false, None);
        assert!(app.confirm.is_none());
        assert_eq!(app.notice, t(app.language, "Ação disponível apenas para pull/merge requests."));
        app.ask(Action::Comment, "Enviar comentário", false, false, Some("oi".into()));
        assert!(app.confirm.as_ref().is_some_and(|confirm| confirm.request.body == "oi" && !confirm.uses_draft));
    }

    #[test]
    fn drafts_follow_their_item() {
        let mut app = demo();
        let keys: Vec<String> = app.visible().iter().map(|row| row.key.clone()).collect();
        app.draft.replace_text("rascunho");
        app.select(&keys[1]);
        assert_eq!(app.draft.text, "");
        app.select(&keys[0]);
        assert_eq!(app.draft.text, "rascunho");
        assert_eq!(app.prefs().drafts.get(&keys[0]).map(String::as_str), Some("rascunho"));
        app.draft.clear();
        assert!(app.prefs().drafts.is_empty(), "an emptied draft is forgotten");
    }

    #[test]
    fn stale_conversations_are_ignored() {
        let mut app = demo();
        let generation = app.detail_generation;
        app.thread = None;
        app.apply(Event::Detail { generation: generation - 1, result: Ok(Thread { body: "old".into(), comments: Vec::new() }) }, 1.0);
        assert!(app.thread.is_none());
        app.apply(Event::Detail { generation, result: Ok(Thread { body: "new".into(), comments: Vec::new() }) }, 1.0);
        assert_eq!(app.thread.map(|thread| thread.body), Some("new".to_string()));
    }

    #[test]
    fn a_dirty_worktree_offers_forced_removal_and_a_missing_clone_opens_settings() {
        let mut app = demo();
        let key = app.selected.clone();
        let request = Request { key: key.clone(), action: Action::Merge, body: String::new(), remove_after: true, force: false, push: false };
        let executed = |request: &Request, error| Event::Executed {
            request: request.clone(),
            uses_draft: false,
            message: String::new(),
            error: Some(error),
            reload_thread: false,
            fresh: Err(msg!(app::ERR_CLOSED)),
        };
        app.apply(executed(&request, msg!(ERR_DIRTY_WORKTREE)), 1.0);
        let confirm = app.confirm.take().expect("offers to discard local changes");
        assert!(confirm.request.force && confirm.request.action == Action::RemoveWorktree && confirm.worktree == "/example/worktree");
        app.apply(executed(&request, msg!(ERR_CLONE_REQUIRED)), 1.0);
        assert!(app.settings.is_some() && !app.notice.is_empty());
    }

    #[test]
    fn settings_are_not_saved_in_demo_mode() {
        let mut app = demo();
        app.open_settings();
        app.save_settings();
        assert!(app.pending.is_empty() && !app.error.is_empty());
        app.toggle_track_all();
        assert!(app.pending.is_empty());
    }

    #[test]
    fn the_palette_filters_commands_and_yields_to_dialogs() {
        let mut app = demo();
        app.toggle_palette();
        assert!(app.palette_open && app.palette_query.focused);
        assert_eq!(app.palette_commands().len(), PaletteCommand::ALL.len());
        app.palette_query.replace_text("MERGE");
        assert_eq!(app.palette_commands().iter().map(|command| command.0).collect::<Vec<_>>(), [PaletteCommand::Merge]);
        app.toggle_palette();
        app.open_settings();
        app.toggle_palette();
        assert!(!app.palette_open, "settings keep the keyboard");
    }

    #[test]
    fn only_web_links_reach_the_browser() {
        let mut app = demo();
        assert_eq!(app.browser_url().as_deref(), Some("https://example.com"));
        let index = app.rows.iter().position(|row| row.key == app.selected).unwrap();
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://", ""] {
            app.rows[index].item.url = bad.into();
            assert_eq!(app.browser_url(), None, "{bad:?}");
        }
    }

    #[test]
    fn refresh_is_scheduled_after_each_result() {
        let mut app = demo();
        assert!(app.refresh_due > 1.0, "next refresh follows the configured interval");
        let due = app.refresh_due;
        assert!(!app.tick(due - 1.0) || !app.refreshing);
        app.tick(due + 0.1);
        assert!(app.refreshing, "the due refresh starts on its own");
        settle(&mut app);
    }
}
