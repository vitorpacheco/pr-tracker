//! The orchestration behind the interface: refreshing instances, reconciling
//! snapshots and running actions on items.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::cache;
use crate::config::{Config, Instance, Repo};
use crate::error::{Error, Result};
use crate::gitops;
use crate::i18n;
use crate::provider::{self, Client, Item};
use crate::toolchain::Ctx;

mod actions;
mod diagnostics;
mod session;

pub use actions::{Action, Command, ERR_CLONE_REQUIRED, ERR_DIRTY_WORKTREE, Outcome};
pub use diagnostics::{Diagnostics, InstanceStatus, ToolStatus};
pub use session::{ERR_BUSY, ERR_CLOSED, Session, State};

/// Where complete snapshots of an instance are kept between runs.
pub trait SnapshotStore: Send + Sync {
    fn load(&self, instance: &Instance) -> Result<cache::Snapshot>;
    fn replace(&self, instance: &Instance, items: &[Item], synced_at: DateTime<Utc>) -> Result<()>;
}

impl SnapshotStore for cache::Store {
    fn load(&self, instance: &Instance) -> Result<cache::Snapshot> {
        cache::Store::load(self, instance)
    }
    fn replace(&self, instance: &Instance, items: &[Item], synced_at: DateTime<Utc>) -> Result<()> {
        cache::Store::replace(self, instance, items, synced_at)
    }
}

type ClientFn = dyn Fn(&Instance, &[Repo]) -> Arc<dyn Client> + Send + Sync;
type ResolveCloneFn = dyn Fn(&Ctx, &Config, &Item) -> Option<(PathBuf, String)> + Send + Sync;
type WorktreeExistsFn = dyn Fn(&Config, &Item) -> Option<PathBuf> + Send + Sync;
type CreateWorktreeFn =
    dyn Fn(&Ctx, &Config, &dyn Client, &Item, &Path, &str) -> Result<PathBuf> + Send + Sync;
type PushWorktreeFn = dyn Fn(&Ctx, &Path, &Item) -> Result<()> + Send + Sync;
type RemoveWorktreeFn = dyn Fn(&Ctx, &Config, &Item, bool) -> Result<()> + Send + Sync;

/// The side effects an [`Application`] performs, replaceable in tests.
#[derive(Clone)]
pub struct Hooks {
    pub client: Arc<ClientFn>,
    pub resolve_clone: Arc<ResolveCloneFn>,
    pub worktree_exists: Arc<WorktreeExistsFn>,
    pub create_worktree: Arc<CreateWorktreeFn>,
    pub push_worktree: Arc<PushWorktreeFn>,
    pub remove_worktree: Arc<RemoveWorktreeFn>,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks {
            client: Arc::new(provider::new),
            resolve_clone: Arc::new(gitops::resolve_clone),
            worktree_exists: Arc::new(gitops::worktree_exists),
            create_worktree: Arc::new(gitops::create_worktree),
            push_worktree: Arc::new(gitops::push_worktree),
            remove_worktree: Arc::new(gitops::remove_worktree),
        }
    }
}

/// Captures configuration for one generation of background work. Recreate it
/// after settings change; in-flight work keeps its original settings.
#[derive(Clone)]
pub struct Application {
    cfg: Config,
    store: Option<Arc<dyn SnapshotStore>>,
    hooks: Hooks,
}

/// Complete snapshots only, including successful empty lists.
#[derive(Debug, Default)]
pub struct Cached {
    pub items: HashMap<String, Vec<Item>>,
    pub synced_at: Option<DateTime<Utc>>,
}

/// Distinguishes remote failures from failures to persist fresh data. A
/// missing instance in `items` preserves its previous items; an empty entry
/// clears them.
#[derive(Debug, Default)]
pub struct RefreshResult {
    pub items: HashMap<String, Vec<Item>>,
    pub errors: HashMap<String, Error>,
    pub cache_errors: HashMap<String, Error>,
    pub clones: HashMap<String, PathBuf>,
}

/// Presentation-independent list state after applying a refresh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub errors: HashMap<String, Error>,
    pub clones: HashMap<String, PathBuf>,
    pub cache_error: Option<Error>,
}

impl Application {
    pub fn new(cfg: &Config, store: Option<Arc<dyn SnapshotStore>>) -> Self {
        Application {
            cfg: cfg.clone(),
            store,
            hooks: Hooks::default(),
        }
    }

    pub fn with_hooks(mut self, hooks: Hooks) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    fn client(&self, instance: &Instance) -> Arc<dyn Client> {
        (self.hooks.client)(instance, &self.cfg.repos)
    }

    fn language(&self) -> &'static str {
        i18n::resolve(&self.cfg.language)
    }

    fn t(&self, message: &str) -> String {
        i18n::text(self.language(), message)
    }

    /// Usable snapshots come back even when another instance's cache fails.
    pub fn load(&self) -> (Cached, Option<Error>) {
        let mut cached = Cached::default();
        let Some(store) = &self.store else {
            return (cached, None);
        };
        let mut errors = Vec::new();
        for instance in self
            .cfg
            .instances
            .iter()
            .filter(|instance| !instance.disabled)
        {
            let snapshot = match store.load(instance) {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    errors.push(err);
                    continue;
                }
            };
            let Some(synced_at) = snapshot.synced_at else {
                continue;
            };
            cached.items.insert(instance.name.clone(), snapshot.items);
            if cached.synced_at.is_none_or(|latest| synced_at > latest) {
                cached.synced_at = Some(synced_at);
            }
        }
        (cached, Error::join(errors))
    }

    /// Queries every enabled instance concurrently.
    pub fn refresh(&self, ctx: &Ctx) -> RefreshResult {
        let ctx = ctx.with_paths(&self.cfg.tool_paths);
        let result = Mutex::new(RefreshResult::default());
        std::thread::scope(|scope| {
            for instance in self
                .cfg
                .instances
                .iter()
                .filter(|instance| !instance.disabled)
            {
                let client = self.client(instance);
                let (ctx, result) = (&ctx, &result);
                scope.spawn(move || {
                    let listed = client.list(ctx);
                    let mut cache_error = None;
                    let mut clones = HashMap::new();
                    if let Ok(items) = &listed {
                        if let Some(store) = &self.store {
                            cache_error = store.replace(instance, items, Utc::now()).err();
                        }
                        for item in items {
                            if let Some((path, _)) =
                                (self.hooks.resolve_clone)(ctx, &self.cfg, item)
                            {
                                clones.insert(item.key(), path);
                            }
                        }
                    }
                    let mut result = result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match listed {
                        Err(err) => {
                            result.errors.insert(instance.name.clone(), err);
                        }
                        Ok(items) => {
                            result.items.insert(instance.name.clone(), items);
                            if let Some(err) = cache_error {
                                result.cache_errors.insert(instance.name.clone(), err);
                            }
                            result.clones.extend(clones);
                        }
                    }
                });
            }
        });
        result
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Preserves stale data on remote failure and drops removed or disabled
    /// instances.
    pub fn reconcile(&self, previous: &Snapshot, result: &RefreshResult) -> Snapshot {
        let mut next = Snapshot::default();
        let mut cache_errors = Vec::new();
        for instance in self
            .cfg
            .instances
            .iter()
            .filter(|instance| !instance.disabled)
        {
            match result.items.get(&instance.name) {
                Some(items) => next.items.extend(items.iter().cloned()),
                None => next.items.extend(
                    previous
                        .items
                        .iter()
                        .filter(|item| item.instance == instance.name)
                        .cloned(),
                ),
            }
            if let Some(err) = result.errors.get(&instance.name) {
                next.errors.insert(instance.name.clone(), err.clone());
            }
            if let Some(err) = result.cache_errors.get(&instance.name) {
                cache_errors.push(err.clone());
            }
        }
        next.items
            .sort_by_key(|item| std::cmp::Reverse(item.updated_at));
        for item in &next.items {
            let key = item.key();
            if let Some(path) = result.clones.get(&key) {
                next.clones.insert(key, path.clone());
            } else if result.errors.contains_key(&item.instance)
                && let Some(path) = previous.clones.get(&key)
            {
                next.clones.insert(key, path.clone());
            }
        }
        next.cache_error = Error::join(cache_errors);
        next
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Provider;
    use crate::provider::{MergeOptions, Thread};
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    type ListFn = dyn Fn(&Ctx) -> Result<Vec<Item>> + Send + Sync;

    /// A provider whose listing is scripted and whose actions are recorded.
    #[derive(Default)]
    pub(crate) struct FakeClient {
        pub instance: Instance,
        pub list: Option<Box<ListFn>>,
        pub calls: Mutex<Vec<String>>,
        pub fail: Mutex<Option<Error>>,
        pub options: Mutex<MergeOptions>,
        pub body: Mutex<String>,
    }

    impl FakeClient {
        pub fn listing(
            list: impl Fn(&Ctx) -> Result<Vec<Item>> + Send + Sync + 'static,
        ) -> Arc<FakeClient> {
            Arc::new(FakeClient {
                list: Some(Box::new(list)),
                ..FakeClient::default()
            })
        }

        fn record(&self, call: impl Into<String>) -> Result<()> {
            self.calls.lock().unwrap().push(call.into());
            self.fail.lock().unwrap().clone().map_or(Ok(()), Err)
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Client for FakeClient {
        fn instance(&self) -> &Instance {
            &self.instance
        }
        fn tool(&self) -> &'static str {
            "fake"
        }
        fn list(&self, ctx: &Ctx) -> Result<Vec<Item>> {
            match &self.list {
                Some(list) => list(ctx),
                None => Ok(Vec::new()),
            }
        }
        fn thread(&self, _: &Ctx, _: &Item) -> Result<Thread> {
            self.record("detail")?;
            Ok(Thread {
                body: "<details>raw markdown</details>".into(),
                comments: Vec::new(),
            })
        }
        fn add_comment(&self, _: &Ctx, _: &Item, body: &str) -> Result<()> {
            *self.body.lock().unwrap() = body.to_string();
            self.record("comment")
        }
        fn approve(&self, _: &Ctx, _: &Item) -> Result<()> {
            self.record("approve")
        }
        fn merge(&self, _: &Ctx, _: &Item, options: &MergeOptions) -> Result<()> {
            *self.options.lock().unwrap() = options.clone();
            self.record("merge")
        }
        fn close(&self, _: &Ctx, _: &Item) -> Result<()> {
            self.record("close")
        }
        fn set_draft(&self, _: &Ctx, _: &Item, draft: bool) -> Result<()> {
            self.record(if draft { "mark_draft" } else { "mark_ready" })
        }
        fn update_branch(&self, _: &Ctx, _: &Item) -> Result<()> {
            self.record("update_branch")
        }
        fn checkout(&self, _: &Ctx, _: &Item, dir: &Path) -> Result<()> {
            self.record(format!("checkout:{}", dir.display()))
        }
        fn head_ref(&self, _: &Item) -> String {
            String::new()
        }
        fn auth_status(&self, _: &Ctx) -> Result<()> {
            self.record("auth")
        }
    }

    pub(crate) fn instance(name: &str) -> Instance {
        Instance {
            name: name.into(),
            ..Instance::default()
        }
    }

    pub(crate) fn hooks_with(
        client: impl Fn(&Instance) -> Arc<dyn Client> + Send + Sync + 'static,
    ) -> Hooks {
        Hooks {
            client: Arc::new(move |instance, _| client(instance)),
            resolve_clone: Arc::new(|_, _, _| None),
            worktree_exists: Arc::new(|_, _| None),
            ..Hooks::default()
        }
    }

    fn item(instance: &str, number: i64) -> Item {
        Item {
            instance: instance.into(),
            number,
            ..Item::default()
        }
    }

    struct FailingStore;
    impl SnapshotStore for FailingStore {
        fn load(&self, _: &Instance) -> Result<cache::Snapshot> {
            Ok(cache::Snapshot::default())
        }
        fn replace(&self, _: &Instance, _: &[Item], _: DateTime<Utc>) -> Result<()> {
            Err(Error::external("disk full"))
        }
    }

    #[test]
    fn load_and_refresh_preserve_failed_instance() {
        let mut cfg = Config::default();
        cfg.instances = vec![
            Instance {
                name: "offline".into(),
                provider: Provider::GitHub,
                host: "github.com".into(),
                ..Instance::default()
            },
            Instance {
                name: "online".into(),
                provider: Provider::GitLab,
                host: "gitlab.com".into(),
                ..Instance::default()
            },
            Instance {
                name: "disabled".into(),
                disabled: true,
                ..Instance::default()
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(cache::Store::open(&dir.path().join("cache.db")).unwrap());
        let synced_at = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        for instance in &cfg.instances {
            store
                .replace(instance, &[item(&instance.name, 1)], synced_at)
                .unwrap();
        }
        let mut hooks = hooks_with(move |instance| {
            assert!(!instance.disabled, "disabled provider constructed");
            let name = instance.name.clone();
            FakeClient::listing(move |_| {
                if name == "offline" {
                    return Err(Error::external("offline"));
                }
                Ok(vec![Item {
                    updated_at: Some(synced_at),
                    ..item(&name, 2)
                }])
            })
        });
        hooks.resolve_clone = Arc::new(|_, _, item| {
            assert_ne!(
                item.instance, "offline",
                "resolved clone for incomplete provider result"
            );
            Some((PathBuf::from("/clone"), "origin".into()))
        });
        let app = Application::new(&cfg, Some(store.clone())).with_hooks(hooks);

        let (loaded, err) = app.load();
        assert!(err.is_none());
        assert_eq!((loaded.items.len(), loaded.synced_at), (2, Some(synced_at)));
        let mut previous = app.reconcile(
            &Snapshot::default(),
            &RefreshResult {
                items: loaded.items.clone(),
                ..RefreshResult::default()
            },
        );
        let stale_key = loaded.items["offline"][0].key();
        previous
            .clones
            .insert(stale_key.clone(), PathBuf::from("/old-clone"));

        let next = app.reconcile(&previous, &app.refresh(&Ctx::background()));
        assert_eq!(
            next.items
                .iter()
                .map(|item| item.number)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert!(next.errors.contains_key("offline"));
        assert_eq!(next.clones[&stale_key], PathBuf::from("/old-clone"));
        let persisted = store.load(&cfg.instances[0]).unwrap();
        assert_eq!(
            (persisted.items[0].number, persisted.synced_at),
            (1, Some(synced_at)),
            "failed refresh replaced cache"
        );
        let persisted = store.load(&cfg.instances[1]).unwrap();
        assert_eq!(
            persisted
                .items
                .iter()
                .map(|item| item.number)
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn refresh_runs_instances_concurrently_and_propagates_cancellation() {
        let mut cfg = Config::default();
        cfg.instances = vec![instance("one"), instance("two")];
        let (started, starts) = mpsc::channel();
        let started = Mutex::new(started);
        let app = Application::new(&cfg, None).with_hooks(hooks_with(move |_| {
            let started = started.lock().unwrap().clone();
            FakeClient::listing(move |ctx| {
                started.send(()).unwrap();
                loop {
                    if let Some(err) = ctx.err() {
                        return Err(err);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = Ctx::background().with_cancel(cancel.clone());
        let worker = std::thread::spawn(move || app.refresh(&ctx));
        for _ in 0..2 {
            starts
                .recv_timeout(Duration::from_secs(5))
                .expect("instances did not start concurrently");
        }
        cancel.store(true, Ordering::SeqCst);
        let result = worker.join().unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.errors.len(), 2);
        assert!(result.errors.values().all(|err| *err == Error::Canceled));
    }

    #[test]
    fn refresh_cache_write_failure_keeps_fresh_items() {
        let mut cfg = Config::default();
        cfg.instances = vec![instance("one")];
        let app =
            Application::new(&cfg, Some(Arc::new(FailingStore))).with_hooks(hooks_with(|_| {
                FakeClient::listing(|_| Ok(vec![item("one", 2)]))
            }));
        let next = app.reconcile(&Snapshot::default(), &app.refresh(&Ctx::background()));
        assert_eq!(
            next.items
                .iter()
                .map(|item| item.number)
                .collect::<Vec<_>>(),
            [2]
        );
        assert!(next.errors.is_empty());
        assert!(next.cache_error.is_some());
    }

    #[test]
    fn reconcile_clears_empty_disabled_and_removed_instances() {
        let mut cfg = Config::default();
        cfg.instances = vec![
            instance("empty"),
            Instance {
                disabled: true,
                ..instance("disabled")
            },
            instance("failed"),
        ];
        let app = Application::new(&cfg, None);
        let mut previous = Snapshot::default();
        for name in ["empty", "disabled", "removed", "failed"] {
            let item = item(name, 1);
            previous
                .clones
                .insert(item.key(), PathBuf::from(format!("/{name}")));
            previous.items.push(item);
        }
        let result = RefreshResult {
            items: HashMap::from([
                ("empty".to_string(), Vec::new()),
                ("disabled".to_string(), vec![item("disabled", 0)]),
            ]),
            errors: HashMap::from([("failed".to_string(), Error::external("offline"))]),
            ..RefreshResult::default()
        };
        let next = app.reconcile(&previous, &result);
        assert_eq!(
            next.items
                .iter()
                .map(|item| item.instance.as_str())
                .collect::<Vec<_>>(),
            ["failed"]
        );
        assert_eq!(next.clones.len(), 1);
    }

    #[test]
    fn application_captures_configuration() {
        let mut cfg = Config::default();
        cfg.instances = vec![instance("original")];
        cfg.repos = vec![Repo {
            instance: "original".into(),
            name: "org/repo".into(),
            track_all: true,
            ..Repo::default()
        }];
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let hooks = Hooks {
            client: Arc::new(move |instance, repos| {
                assert!(
                    instance.name == "original" && repos[0].track_all,
                    "configuration changed"
                );
                let seen = seen.clone();
                FakeClient::listing(move |_| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(Vec::new())
                })
            }),
            ..hooks_with(|_| unreachable!())
        };
        let app = Application::new(&cfg, None).with_hooks(hooks);
        cfg.instances[0].name = "edited".into();
        cfg.repos[0].track_all = false;

        let (loaded, err) = app.load();
        assert!(err.is_none() && loaded.items.is_empty());
        let result = app.refresh(&Ctx::background());
        assert!(
            result.items.contains_key("original"),
            "missing empty successful result"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct PartialStore;
    impl SnapshotStore for PartialStore {
        fn load(&self, instance: &Instance) -> Result<cache::Snapshot> {
            match instance.name.as_str() {
                "broken" => Err(Error::external("invalid cached payload")),
                "missing" => Ok(cache::Snapshot::default()),
                _ => Ok(cache::Snapshot {
                    items: Vec::new(),
                    synced_at: Utc.timestamp_opt(100, 0).single(),
                }),
            }
        }
        fn replace(&self, _: &Instance, _: &[Item], _: DateTime<Utc>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn load_returns_usable_snapshots_alongside_cache_errors() {
        let mut cfg = Config::default();
        cfg.instances = vec![instance("broken"), instance("missing"), instance("empty")];
        let app = Application::new(&cfg, Some(Arc::new(PartialStore)));
        let (loaded, err) = app.load();
        assert!(err.is_some(), "cache read failure was hidden");
        assert_eq!(loaded.items.len(), 1);
        assert!(
            loaded.items.contains_key("empty"),
            "successful empty snapshot was discarded"
        );
        assert_eq!(loaded.synced_at, Utc.timestamp_opt(100, 0).single());
    }
}
