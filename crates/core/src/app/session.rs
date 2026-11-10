use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{
    Action, Application, Command, ERR_DIRTY_WORKTREE, Outcome, RefreshResult, Snapshot,
    SnapshotStore,
};
use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::provider::{Item, MergeOptions, Thread};
use crate::toolchain::Ctx;
use crate::{gitops, msg, theme};

/// Catalog key of the error returned once the session is closed.
pub const ERR_CLOSED: &str = "aplicação encerrada";
/// Catalog key of the error for a second action on an item that has one running.
pub const ERR_BUSY: &str = "já existe uma ação em andamento para este item";

/// A detached snapshot. Interfaces own selection and rendering only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub items: Vec<Item>,
    pub errors: HashMap<String, Error>,
    pub clones: HashMap<String, PathBuf>,
    pub cache_error: Option<Error>,
    pub worktrees: HashMap<String, PathBuf>,
    pub synced_at: Option<DateTime<Utc>>,
    pub revision: u64,
}

impl State {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            items: self.items.clone(),
            errors: self.errors.clone(),
            clones: self.clones.clone(),
            cache_error: self.cache_error.clone(),
        }
    }

    fn adopt(&mut self, snapshot: Snapshot) {
        self.items = snapshot.items;
        self.errors = snapshot.errors;
        self.clones = snapshot.clones;
        self.cache_error = snapshot.cache_error;
    }
}

struct Flight {
    id: u64,
    revision: u64,
}

struct Inner {
    application: Arc<Application>,
    state: State,
    loaded: bool,
    revision: u64,
    flight: Option<Flight>,
    /// Flights run one at a time, so the last finished id orders them all.
    finished: u64,
    busy: HashSet<String>,
    /// Items whose worktree removal was refused as dirty, kept so the removal
    /// can still be forced after the item leaves the list.
    cleanup: HashMap<String, Item>,
    workers: usize,
    closed: bool,
}

impl Inner {
    fn scan_worktrees(&mut self) {
        let application = self.application.clone();
        self.state.worktrees = self
            .state
            .items
            .iter()
            .filter_map(|item| {
                (application.hooks.worktree_exists)(&application.cfg, item)
                    .map(|dir| (item.key(), dir))
            })
            .collect();
    }

    fn item(&self, key: &str) -> Result<Item> {
        self.state
            .items
            .iter()
            .find(|item| item.key() == key)
            .cloned()
            .ok_or_else(|| msg!("item não encontrado; atualize a lista"))
    }

    fn configure(&mut self, cfg: &Config) {
        let next = Arc::new(Application {
            cfg: cfg.clone(),
            store: self.application.store.clone(),
            hooks: self.application.hooks.clone(),
        });
        self.revision += 1;
        self.loaded = false;
        // Never retain cached items under a changed host/provider identity.
        let items = std::mem::take(&mut self.state.items)
            .into_iter()
            .filter(|item| {
                cfg.instance(&item.instance).is_some_and(|instance| {
                    !instance.disabled
                        && instance.host == item.host
                        && instance.provider == item.provider
                })
            })
            .collect();
        let snapshot = next.reconcile(
            &Snapshot {
                items,
                ..Snapshot::default()
            },
            &RefreshResult::default(),
        );
        self.application = next;
        self.state.adopt(snapshot);
        self.state.revision = self.revision;
        self.scan_worktrees();
    }
}

/// Owns application state and lifetime. Concurrent refresh callers share one
/// operation; a caller that gives up does not cancel it for the others.
pub struct Session {
    inner: Mutex<Inner>,
    changed: Condvar,
    cancel: Arc<AtomicBool>,
}

/// Marks a unit of background work so [`Session::close`] can wait for it.
pub(super) struct Worker<'a>(&'a Session);

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        self.0.lock().workers -= 1;
        self.0.changed.notify_all();
    }
}

impl Session {
    pub fn new(cfg: &Config, store: Option<Arc<dyn SnapshotStore>>) -> Arc<Session> {
        Self::with_application(Application::new(cfg, store))
    }

    pub fn with_application(application: Application) -> Arc<Session> {
        Arc::new(Session {
            inner: Mutex::new(Inner {
                application: Arc::new(application),
                state: State::default(),
                loaded: false,
                revision: 0,
                flight: None,
                finished: 0,
                busy: HashSet::new(),
                cleanup: HashMap::new(),
                workers: 0,
                closed: false,
            }),
            changed: Condvar::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The current application and a marker for the work about to use it, or
    /// `None` once the session is closed.
    pub(super) fn begin_work(&self) -> Option<(Arc<Application>, Worker<'_>)> {
        let mut inner = self.lock();
        if inner.closed {
            return None;
        }
        inner.workers += 1;
        Some((inner.application.clone(), Worker(self)))
    }

    /// A context bound to the session's lifetime and the caller's own.
    pub(super) fn work(&self, caller: &Ctx, timeout: Duration) -> Ctx {
        caller
            .with_cancel(self.cancel.clone())
            .with_timeout(timeout)
    }

    /// Cancels running work and waits for it to finish.
    pub fn close(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let mut inner = self.lock();
        inner.closed = true;
        self.changed.notify_all();
        while inner.workers > 0 {
            inner = self
                .changed
                .wait(inner)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub fn configuration(&self) -> Config {
        self.lock().application.cfg.clone()
    }

    /// Commits a validated copy before changing live state. Failure leaves
    /// both the running configuration and its generation untouched.
    pub fn save_settings(&self, cfg: &Config) -> Result<()> {
        let mut inner = self.lock();
        if inner.closed {
            return Err(msg!(ERR_CLOSED));
        }
        let current = &inner.application.cfg;
        let mut next = current.clone();
        next.language = cfg.language.clone();
        next.theme_file = cfg.theme_file.clone();
        if let Some(path) = next.theme_path() {
            theme::resolve(Some(&path))?;
        }
        next.tool_paths = cfg.tool_paths.clone();
        next.desktop_terminal = cfg.desktop_terminal.clone();
        next.refresh_interval = cfg.refresh_interval.clone();
        next.terminal = cfg.terminal.clone();
        next.diff_tool = cfg.diff_tool.clone();
        next.worktree_dir = cfg.worktree_dir.clone();
        next.clone_roots = cfg.clone_roots.clone();
        next.instances = cfg.instances.clone();
        next.repos = cfg.repos.clone();
        for instance in &mut next.instances {
            if instance.host.is_empty() {
                instance.host = instance.provider.default_host().to_string();
            }
            instance.host = config::normalize_host(&instance.host);
            if instance.name.is_empty() {
                instance.name = instance.host.clone();
            }
        }
        next.validate()?;
        for repo in &next.repos {
            if next.instance(&repo.instance).is_none() {
                return Err(msg!(
                    "repositório referencia uma instância inexistente: %s",
                    &repo.instance
                ));
            }
            if repo.name.is_empty() {
                return Err(msg!("nome do repositório é obrigatório"));
            }
            if !config::valid_merge_method(&repo.merge_method) {
                return Err(msg!("método de merge inválido no repositório"));
            }
            let unchanged = current
                .repo(&repo.instance, &repo.name)
                .is_some_and(|previous| previous.path == repo.path);
            if !repo.path.is_empty() && !unchanged {
                let ctx = self
                    .work(&Ctx::background(), Duration::from_secs(15))
                    .with_paths(&next.tool_paths);
                gitops::validate_clone(&ctx, &config::expand_home(&repo.path))?;
            }
        }
        next.save()?;
        inner.configure(&next);
        Ok(())
    }

    /// The current state, reading the cache on first use.
    pub fn load(&self, ctx: &Ctx) -> Result<State> {
        let mut inner = self.lock();
        if inner.closed {
            return Err(msg!(ERR_CLOSED));
        }
        ctx.check()?;
        if !inner.loaded {
            let application = inner.application.clone();
            let (mut cached, err) = application.load();
            for item in &inner.state.items {
                cached.items.remove(&item.instance);
            }
            let snapshot = application.reconcile(
                &inner.state.snapshot(),
                &RefreshResult {
                    items: cached.items,
                    ..RefreshResult::default()
                },
            );
            inner.state.adopt(snapshot);
            inner.state.cache_error = err;
            if cached.synced_at > inner.state.synced_at {
                inner.state.synced_at = cached.synced_at;
            }
            inner.state.revision = inner.revision;
            inner.scan_worktrees();
            inner.loaded = true;
        }
        Ok(inner.state.clone())
    }

    /// Queries the providers, or joins the query already running.
    pub fn refresh(self: &Arc<Self>, ctx: &Ctx) -> Result<State> {
        self.load(ctx)?;
        loop {
            let mut inner = self.lock();
            if inner.closed {
                return Err(msg!(ERR_CLOSED));
            }
            ctx.check()?;
            let (id, revision) = match &inner.flight {
                Some(flight) => (flight.id, flight.revision),
                None => {
                    let flight = Flight {
                        id: inner.finished + 1,
                        revision: inner.revision,
                    };
                    let (id, revision) = (flight.id, flight.revision);
                    inner.flight = Some(flight);
                    inner.workers += 1;
                    let (session, application) = (self.clone(), inner.application.clone());
                    std::thread::spawn(move || session.run_refresh(&application, id, revision));
                    (id, revision)
                }
            };
            while inner.finished < id {
                if inner.closed {
                    return Err(msg!(ERR_CLOSED));
                }
                ctx.check()?;
                inner = self
                    .changed
                    .wait_timeout(inner, Duration::from_millis(20))
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
            if inner.closed {
                return Err(msg!(ERR_CLOSED));
            }
            if revision == inner.revision {
                return Ok(inner.state.clone());
            }
            // Settings changed while the old generation was querying providers.
        }
    }

    fn run_refresh(&self, application: &Application, id: u64, revision: u64) {
        let _worker = Worker(self);
        let ctx = self.work(&Ctx::background(), Duration::from_secs(120));
        let result = application.refresh(&ctx);
        let mut inner = self.lock();
        if !inner.closed && revision == inner.revision {
            let snapshot = application.reconcile(&inner.state.snapshot(), &result);
            inner.state.adopt(snapshot);
            if !result.items.is_empty() {
                inner.state.synced_at = Some(Utc::now());
            }
            inner.state.revision = inner.revision;
            inner.loaded = true;
            inner.scan_worktrees();
        }
        inner.flight = None;
        inner.finished = id;
    }

    pub fn detail(&self, ctx: &Ctx, key: &str) -> Result<Thread> {
        ctx.check()?;
        let (item, application) = {
            let mut inner = self.lock();
            if inner.closed {
                return Err(msg!(ERR_CLOSED));
            }
            let item = inner.item(key)?;
            inner.workers += 1;
            (item, inner.application.clone())
        };
        let _worker = Worker(self);
        application.detail(&self.work(ctx, Duration::from_secs(60)), &item)
    }

    /// Resolves the item from trusted state, so a caller cannot substitute a
    /// different repository or URL in a command.
    pub fn execute(&self, ctx: &Ctx, key: &str, mut command: Command) -> Outcome {
        let failed = |error| Outcome {
            error: Some(error),
            ..Outcome::default()
        };
        if let Some(err) = ctx.err() {
            return failed(err);
        }
        let application = {
            let mut inner = self.lock();
            if inner.closed {
                return failed(msg!(ERR_CLOSED));
            }
            let pending = (command.action == Action::RemoveWorktree)
                .then(|| inner.cleanup.get(key).cloned())
                .flatten();
            command.item = match inner.item(key) {
                Ok(item) => item,
                Err(err) => match pending {
                    Some(item) => item,
                    None => return failed(err),
                },
            };
            if !inner.busy.insert(key.to_string()) {
                return failed(msg!(ERR_BUSY));
            }
            inner.workers += 1;
            inner.application.clone()
        };
        let _worker = Worker(self);
        let out = application.execute(&self.work(ctx, Duration::from_secs(300)), &command);
        let mut inner = self.lock();
        let dirty = out
            .error
            .as_ref()
            .is_some_and(|err| err.is(ERR_DIRTY_WORKTREE));
        if dirty && (command.action == Action::RemoveWorktree || command.remove_after) {
            inner.cleanup.insert(key.to_string(), command.item.clone());
        }
        if command.action == Action::RemoveWorktree && out.error.is_none() {
            inner.cleanup.remove(key);
        }
        inner.busy.remove(key);
        inner.scan_worktrees();
        out
    }

    pub fn merge_options(&self, key: &str) -> Result<MergeOptions> {
        let inner = self.lock();
        if inner.closed {
            return Err(msg!(ERR_CLOSED));
        }
        Ok(inner.application.merge_options(&inner.item(key)?))
    }
}

#[cfg(test)]
mod tests {
    use super::super::actions::tests::fixture;
    use super::super::tests::{FakeClient, hooks_with};
    use super::super::{Action, Application, ERR_DIRTY_WORKTREE, Hooks};
    use super::*;
    use crate::config::{CommentPreset, Instance, Provider};
    use crate::i18n::tests::env_lock;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: every test that reads or writes the environment holds env_lock.
        unsafe { std::env::set_var(key, value) }
    }

    fn loaded(application: Application, items: Vec<Item>) -> Arc<Session> {
        let session = Session::with_application(application);
        {
            let mut inner = session.lock();
            inner.loaded = true;
            inner.state.items = items;
        }
        session
    }

    /// A listing that reports its start and then waits to be released.
    fn gated(item: Item, calls: Arc<AtomicUsize>) -> (Hooks, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started, starts) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let (started, released) = (Mutex::new(started), Arc::new(Mutex::new(released)));
        let hooks = hooks_with(move |_| {
            let (item, calls, started, released) = (
                item.clone(),
                calls.clone(),
                started.lock().unwrap().clone(),
                released.clone(),
            );
            FakeClient::listing(move |ctx| {
                calls.fetch_add(1, Ordering::SeqCst);
                let _ = started.send(());
                loop {
                    if released
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_millis(2))
                        .is_ok()
                    {
                        return Ok(vec![item.clone()]);
                    }
                    ctx.check()?;
                }
            })
        });
        (hooks, starts, release)
    }

    #[test]
    fn session_shares_refresh_and_returns_detached_state() {
        let (cfg, _, _, mut item) = fixture();
        item.labels = vec!["original".into()];
        let calls = Arc::new(AtomicUsize::new(0));
        let (hooks, starts, release) = gated(item.clone(), calls.clone());
        let session = loaded(Application::new(&cfg, None).with_hooks(hooks), Vec::new());
        let background = session.clone();
        let first = std::thread::spawn(move || background.refresh(&Ctx::background()));
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        // A caller that gives up must not cancel the shared refresh.
        let canceled = Ctx::background().with_cancel(Arc::new(AtomicBool::new(true)));
        assert_eq!(session.refresh(&canceled), Err(Error::Canceled));
        release.send(()).unwrap();
        let mut state = first.join().unwrap().unwrap();
        assert_eq!((calls.load(Ordering::SeqCst), state.items.len()), (1, 1));
        state.items[0].labels[0] = "mutated".into();
        assert_eq!(
            session.load(&Ctx::background()).unwrap().items[0].labels,
            ["original"],
            "state escaped"
        );
        session.close();
    }

    #[test]
    fn session_discards_old_generation_and_closes_work() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let (cfg, _, _, item) = fixture();
        let (hooks, starts, release) = gated(item, Arc::new(AtomicUsize::new(0)));
        let session = loaded(Application::new(&cfg, None).with_hooks(hooks), Vec::new());
        let background = session.clone();
        let refresh = std::thread::spawn(move || background.refresh(&Ctx::background()));
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut next = session.configuration();
        next.instances.clear();
        next.repos.clear();
        session.save_settings(&next).unwrap();
        release.send(()).unwrap();
        let state = refresh.join().unwrap().unwrap();
        assert!(
            state.items.is_empty() && state.revision == 1,
            "old result accepted: {state:?}"
        );
        session.close();
        assert!(
            session
                .refresh(&Ctx::background())
                .is_err_and(|err| err.is(ERR_CLOSED))
        );
    }

    #[test]
    fn session_resolves_command_item_and_serializes_item_actions() {
        let (cfg, hooks, client, item) = fixture();
        let session = loaded(
            Application::new(&cfg, None).with_hooks(hooks),
            vec![item.clone()],
        );
        let spoofed = Item {
            repo: "other/repo".into(),
            ..item.clone()
        };
        let command = Command {
            item: spoofed,
            body: "hello".into(),
            ..Command::new(Action::Comment)
        };
        let out = session.execute(&Ctx::background(), &item.key(), command);
        assert_eq!(
            (out.error, out.message.as_str()),
            (None, "comentário enviado em org/repo#7")
        );
        assert_eq!(*client.body.lock().unwrap(), "hello");
        session.lock().busy.insert(item.key());
        let out = session.execute(
            &Ctx::background(),
            &item.key(),
            Command::new(Action::Approve),
        );
        assert!(out.error.is_some_and(|err| err.is(ERR_BUSY)));
        assert!(
            session.detail(&Ctx::background(), "missing").is_err(),
            "unknown item accepted"
        );
    }

    #[test]
    fn save_settings_rollback_and_isolation() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let session = Session::new(&Config::default(), None);
        let mut next = session.configuration();
        next.instances = vec![Instance {
            provider: Provider::GitHub,
            ..Instance::default()
        }];
        session.save_settings(&next).unwrap();
        assert_eq!(session.configuration().instances[0].name, "github.com");
        let mut next = session.configuration();
        next.terminal = "invalid".into();
        assert!(
            session.save_settings(&next).is_err(),
            "invalid settings accepted"
        );
        assert_eq!(
            session.configuration().terminal,
            "auto",
            "failed settings changed live config"
        );
        assert_eq!(Config::load().unwrap().0.terminal, "auto");
    }

    #[test]
    fn comment_preset_settings_are_persisted_and_validated() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let mut cfg = Config::default();
        let preset = CommentPreset {
            label: "Aprovado".into(),
            body: "aprovado".into(),
        };
        cfg.instances = vec![Instance {
            name: "work".into(),
            provider: Provider::GitLab,
            comment_presets: vec![preset],
            ..Instance::default()
        }];
        let session = Session::new(&cfg, None);
        let mut next = session.configuration();
        next.instances[0].comment_presets[0].body = "aprovado\n\nTestes verificados.".into();
        session.save_settings(&next).unwrap();
        let mut invalid = session.configuration();
        invalid.instances[0].comment_presets[0].body = " ".into();
        assert!(
            session.save_settings(&invalid).is_err(),
            "blank preset accepted"
        );
        let live = session.configuration().instances[0].comment_presets[0]
            .body
            .clone();
        assert_eq!(live, "aprovado\n\nTestes verificados.");
        assert_eq!(
            Config::load().unwrap().0.instances[0].comment_presets[0].body,
            live
        );
    }

    #[test]
    fn dirty_cleanup_remains_possible_after_closed_item_disappears() {
        let (cfg, mut hooks, _, item) = fixture();
        hooks.remove_worktree = Arc::new(|_, _, _, force| {
            if force {
                Ok(())
            } else {
                Err(msg!(ERR_DIRTY_WORKTREE))
            }
        });
        let session = loaded(
            Application::new(&cfg, None).with_hooks(hooks),
            vec![item.clone()],
        );
        let out = session.execute(
            &Ctx::background(),
            &item.key(),
            Command {
                remove_after: true,
                ..Command::new(Action::Close)
            },
        );
        assert!(out.refresh && out.error.is_some_and(|err| err.is(ERR_DIRTY_WORKTREE)));
        // Closed PRs are absent from the next remote listing.
        session.lock().state.items.clear();
        let force = || Command {
            force: true,
            ..Command::new(Action::RemoveWorktree)
        };
        assert_eq!(
            session
                .execute(&Ctx::background(), &item.key(), force())
                .error,
            None
        );
        assert!(
            session
                .execute(&Ctx::background(), &item.key(), force())
                .error
                .is_some(),
            "completed cleanup remained authorized"
        );
    }

    #[test]
    fn dirty_update_does_not_schedule_cleanup() {
        let (cfg, mut hooks, _, item) = fixture();
        hooks.create_worktree = Arc::new(|_, _, _, _, _, _| Err(msg!(ERR_DIRTY_WORKTREE)));
        let session = loaded(
            Application::new(&cfg, None).with_hooks(hooks),
            vec![item.clone()],
        );
        let out = session.execute(
            &Ctx::background(),
            &item.key(),
            Command::new(Action::UpdateWorktree),
        );
        assert!(
            out.error
                .as_ref()
                .is_some_and(|err| err.is(ERR_DIRTY_WORKTREE))
        );
        assert!(
            out.message.is_empty() && out.directory.is_none() && !out.refresh && !out.reload_thread
        );
        assert!(
            session.lock().cleanup.is_empty(),
            "failed update scheduled worktree cleanup"
        );
    }

    #[test]
    fn save_settings_write_failure_leaves_live_config_unchanged() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("not-a-directory");
        std::fs::write(&parent, "keep").unwrap();
        set_env("PR_TRACKER_CONFIG", parent.join("config.toml"));
        let session = Session::new(&Config::default(), None);
        let mut cfg = session.configuration();
        cfg.refresh_interval = "1m".into();
        assert!(
            session.save_settings(&cfg).is_err(),
            "write to file parent unexpectedly succeeded"
        );
        assert_eq!(session.configuration().refresh_interval, "5m");
        assert_eq!(
            session.lock().revision,
            0,
            "failed write changed the live generation"
        );
    }

    #[test]
    fn session_coalesces_waiters_and_cancels_on_close() {
        let (cfg, _, _, item) = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let (hooks, starts, release) = gated(item, calls.clone());
        let session = loaded(Application::new(&cfg, None).with_hooks(hooks), Vec::new());
        let waiters: Vec<_> = (0..4)
            .map(|_| {
                let session = session.clone();
                std::thread::spawn(move || session.refresh(&Ctx::background()))
            })
            .collect();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "concurrent refresh calls");
        release.send(()).unwrap();
        for waiter in waiters {
            waiter.join().unwrap().unwrap();
        }
        // A second operation must be interrupted by the session's lifetime.
        let background = session.clone();
        let refresh = std::thread::spawn(move || background.refresh(&Ctx::background()));
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        session.close();
        assert!(refresh.join().unwrap().is_err_and(|err| err.is(ERR_CLOSED)));
        drop(release);
    }

    #[test]
    fn save_custom_theme_and_reject_missing_file() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        theme::export(&dir.path().join("colors.toml"), &theme::default_dark()).unwrap();
        let session = Session::new(&Config::default(), None);
        let mut next = session.configuration();
        next.theme_file = "colors.toml".into();
        session.save_settings(&next).unwrap();
        next.theme_file = "missing.toml".into();
        assert!(
            session.save_settings(&next).is_err(),
            "accepted missing palette"
        );
        assert_eq!(
            session.configuration().theme_file,
            "colors.toml",
            "failed save changed live theme"
        );
        assert_eq!(Config::load().unwrap().0.theme_file, "colors.toml");
    }
}
