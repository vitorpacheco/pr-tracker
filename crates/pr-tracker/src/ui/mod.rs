//! The window: a Rune application over the shared session.

use std::sync::Arc;
use std::time::Duration;

use pr_tracker_core::app::Action;
use pr_tracker_core::provider::Kind;
use rune::{AppSpec, Key, KeyEvent, Tx, Waker};
use rune_kit::shell;

mod demo;
mod dialogs;
mod inspector;
mod list;
mod prefs;
mod settings;
mod state;
mod text;
mod theme;
mod view;
mod widgets;

pub use state::App;
use state::{DetailTab, PaletteCommand};

const TITLE: &str = "pr-tracker";
const SIZE: (f32, f32) = (1280.0, 820.0);
const MIN_SIZE: (f32, f32) = (760.0, 480.0);

fn demo_requested() -> bool {
    rune::extra_args().iter().any(|arg| arg == "--demo")
}

fn init() -> App {
    if !demo_requested() {
        return App::open();
    }
    let mut app = App::demo();
    if rune::tooling::headless_run().is_some() {
        // A headless frame is drawn on a virtual clock that does not wait for
        // threads, so the fictional data is loaded before the first one.
        app.begin(Arc::new(|| {}));
        app.settle();
    }
    app
}

fn setup(app: &mut App, waker: Waker) {
    if !app.started() {
        app.begin(waker);
    }
}

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let changed = app.tick(clock);
    let due = clock + app.next_wake(clock);
    if (due - app.wake_at).abs() > 0.01 {
        app.wake_at = due;
        tx.wake_after(Duration::from_secs_f64(due - clock));
    }
    changed
}

fn close(app: &mut App, _: &mut Tx) {
    app.close();
}

/// Runs a palette command or its keyboard shortcut.
pub(crate) fn run_command(app: &mut App, command: PaletteCommand, tx: &mut Tx) {
    app.palette_open = false;
    match command {
        PaletteCommand::Refresh => app.refresh(),
        PaletteCommand::Search => app.query.focused = true,
        PaletteCommand::Conversation => app.show_conversation(false),
        PaletteCommand::Comment => app.show_conversation(true),
        PaletteCommand::Browser => open_browser(app, tx),
        PaletteCommand::Worktree => app.ask(Action::UpdateWorktree, "Criar ou atualizar worktree", false, false, None),
        PaletteCommand::Terminal => app.ask(Action::PrepareTerminal, "Abrir terminal no worktree", false, false, None),
        PaletteCommand::Diff => app.ask(Action::PrepareDiff, "Abrir diff no terminal", false, false, None),
        PaletteCommand::Approve => app.ask(Action::Approve, "Aprovar pull request", false, false, None),
        PaletteCommand::Merge => app.ask(Action::Merge, "Fazer merge", false, false, None),
        PaletteCommand::Settings => app.open_settings(),
    }
}

pub(crate) fn open_browser(app: &mut App, tx: &mut Tx) {
    app.more_open = false;
    let Some(url) = app.browser_url() else { return };
    if tx.open_url(&url).is_err() {
        app.error = app.tr("navegador indisponível");
    }
}

fn move_selection(app: &mut App, delta: isize, tx: &mut Tx) {
    if let Some(key) = app.move_selection(delta) {
        tx.scroll_into_view(view::row_key(&key));
    }
}

fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.is_command('k') {
        app.toggle_palette();
        return true;
    }
    if app.modal() {
        return false;
    }
    if shell::is_sidebar_toggle(event) {
        app.sidebar.toggle();
        return true;
    }
    if event.is_command(',') {
        app.open_settings();
        return true;
    }
    if event.mods.cmd || event.mods.ctrl || event.mods.alt {
        return false;
    }
    match &event.key {
        Key::Escape => {
            app.more_open = false;
            tx.blur();
        }
        Key::Down => move_selection(app, 1, tx),
        Key::Up => move_selection(app, -1, tx),
        Key::Enter => {
            if !app.inspector.is_open() {
                app.inspector.toggle();
            }
        }
        Key::Text(typed) => match typed.as_str() {
            "j" => move_selection(app, 1, tx),
            "k" => move_selection(app, -1, tx),
            "/" => run_command(app, PaletteCommand::Search, tx),
            "r" => run_command(app, PaletteCommand::Refresh, tx),
            "v" => run_command(app, PaletteCommand::Conversation, tx),
            "n" => run_command(app, PaletteCommand::Comment, tx),
            "o" => run_command(app, PaletteCommand::Browser, tx),
            "a" => run_command(app, PaletteCommand::Approve, tx),
            "m" => run_command(app, PaletteCommand::Merge, tx),
            "t" => run_command(app, PaletteCommand::Terminal, tx),
            "d" => run_command(app, PaletteCommand::Diff, tx),
            "w" => run_command(app, PaletteCommand::Worktree, tx),
            "c" => app.ask(Action::Checkout, "Trocar branch do clone local", false, false, None),
            "X" => app.ask(Action::Close, "Fechar sem merge", false, false, None),
            "x" => app.ask(Action::RemoveWorktree, "Remover worktree", false, false, None),
            "1" => app.switch_kind(Kind::Pr),
            "2" => app.switch_kind(Kind::Issue),
            "s" => app.open_settings(),
            "?" => app.toggle_palette(),
            _ => return false,
        },
        _ => return false,
    }
    true
}

fn spec() -> AppSpec<App> {
    AppSpec {
        min_size: Some(MIN_SIZE),
        families: ("Inter", "ui-monospace"),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: &[
            ("issues", |app| app.switch_kind(Kind::Issue)),
            ("conversation", |app| app.detail_tab = DetailTab::Conversation),
            ("checks", |app| app.detail_tab = DetailTab::Checks),
            ("settings", |app| app.open_settings()),
            ("palette", |app| app.toggle_palette()),
            ("confirm-merge", |app| app.ask(Action::Merge, "Fazer merge", false, false, None)),
            ("confirm-worktree", |app| app.ask(Action::UpdateWorktree, "Criar ou atualizar worktree", false, false, None)),
            ("more", |app| app.more_open = true),
        ],
        ..AppSpec::new(TITLE, SIZE, init, view::view)
    }
}

/// Opens the window and returns when it closes.
pub fn run() {
    rune::set_app_id("io.github.vitorpacheco.pr_tracker");
    rune::App::new(spec()).on_close(close).run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Mods, Runtime};

    fn runtime() -> Runtime<App> {
        let app = state::tests::demo();
        let mut runtime = Runtime::new(app, view::view, SIZE, Env { system_dark: true, scale: 1.0 });
        runtime.on_key = Some(on_key);
        runtime.poll = Some(poll);
        runtime.frame(0.0);
        runtime
    }

    fn frames(runtime: &mut Runtime<App>, count: usize) {
        for _ in 0..count {
            runtime.frame(1.0 / 60.0);
        }
    }

    fn press(runtime: &mut Runtime<App>, key: &str) {
        let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("no element keyed {key:?}"));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        frames(runtime, 2);
    }

    fn typed(text: &str) -> KeyEvent {
        KeyEvent { key: Key::Text(text.into()), mods: Mods::default() }
    }

    #[test]
    fn the_window_draws_the_list_and_the_selected_item() {
        let runtime = runtime();
        let first = runtime.state.visible()[0].key.clone();
        assert!(runtime.find(&view::row_key(&first)).is_some(), "first row is drawn");
        assert!(runtime.find("inspector-title").is_some(), "inspector shows the selection");
        assert!(runtime.find("confirm-dialog").is_none());
    }

    #[test]
    fn pressing_a_row_selects_it() {
        let mut runtime = runtime();
        let third = runtime.state.visible()[2].key.clone();
        press(&mut runtime, &view::row_key(&third));
        assert_eq!(runtime.state.selected, third);
    }

    #[test]
    fn approve_asks_first_and_cancel_runs_nothing() {
        let mut runtime = runtime();
        press(&mut runtime, "action-approve");
        assert!(runtime.state.confirm.is_some());
        assert!(runtime.find("confirm-dialog").is_some(), "the question is on screen");
        press(&mut runtime, "confirm-cancel");
        assert!(runtime.state.confirm.is_none() && runtime.state.busy.is_empty());
    }

    #[test]
    fn confirming_runs_the_action_and_reports_the_result() {
        let mut runtime = runtime();
        press(&mut runtime, "action-approve");
        press(&mut runtime, "confirm-accept");
        assert!(runtime.state.confirm.is_none());
        // The action runs on a thread; frames poll for its result.
        for _ in 0..500 {
            frames(&mut runtime, 1);
            if runtime.state.busy.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        frames(&mut runtime, 2);
        assert!(runtime.state.error.contains("Demo") || runtime.state.error.contains("Demonstração"), "{}", runtime.state.error);
        assert!(runtime.find("banner-error").is_some(), "the failure is shown");
    }

    #[test]
    fn keyboard_shortcuts_drive_the_list_and_dialogs() {
        let mut runtime = runtime();
        let keys: Vec<String> = runtime.state.visible().iter().map(|row| row.key.clone()).collect();
        runtime.key(typed("j"));
        frames(&mut runtime, 2);
        assert_eq!(runtime.state.selected, keys[1]);
        runtime.key(typed("k"));
        frames(&mut runtime, 2);
        assert_eq!(runtime.state.selected, keys[0]);
        runtime.key(typed("v"));
        frames(&mut runtime, 2);
        assert_eq!(runtime.state.detail_tab, DetailTab::Conversation);
        runtime.key(typed("m"));
        frames(&mut runtime, 2);
        assert!(runtime.state.confirm.is_some(), "m asks to merge");
        runtime.key(typed("j"));
        frames(&mut runtime, 2);
        assert_eq!(runtime.state.selected, keys[0], "shortcuts wait while a dialog is open");
    }

    #[test]
    fn kind_and_view_filters_are_reachable_by_pointer() {
        let mut runtime = runtime();
        press(&mut runtime, "kind-issues");
        assert_eq!(runtime.state.kind, Kind::Issue);
        press(&mut runtime, "kind-prs");
        press(&mut runtime, "filter-2");
        assert_eq!(runtime.state.visible().len(), 2, "Mine shows the authored pull requests");
    }

    #[test]
    fn settings_open_with_the_current_configuration() {
        let mut runtime = runtime();
        press(&mut runtime, "open-settings");
        let form = runtime.state.settings.as_ref().expect("settings dialog state");
        assert_eq!(form.instances.len(), 2);
        assert!(runtime.find("settings-save").is_some(), "the dialog is drawn");
    }
}
