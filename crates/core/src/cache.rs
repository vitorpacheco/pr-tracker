//! Persists disposable snapshots of provider items in SQLite. The remote
//! providers remain the source of truth; a cache can always be deleted and
//! rebuilt by the next successful refresh.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Instance;
use crate::error::{Error, Result};
use crate::msg;
use crate::provider::Item;

const APP_NAME: &str = "pr-tracker";
const PATH_ENV: &str = "PR_TRACKER_CACHE";

/// Version 1 belongs to the Go implementation, whose item payloads this
/// version cannot decode. Older databases are emptied rather than converted.
const SCHEMA_VERSION: i64 = 2;

/// The last complete, successful refresh of one instance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub items: Vec<Item>,
    /// `None` when no snapshot is stored.
    pub synced_at: Option<DateTime<Utc>>,
}

/// Owns the SQLite cache. It is safe for concurrent callers.
pub struct Store {
    // One connection is enough for this small local cache and makes per-
    // connection SQLite pragmas deterministic. WAL still permits another
    // pr-tracker process to read while this one commits a snapshot.
    conn: Mutex<Connection>,
}

/// The cache database path. PR_TRACKER_CACHE overrides it.
pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(PATH_ENV).filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    Ok(user_cache_dir()?.join(APP_NAME).join("cache.db"))
}

/// The per-user cache root, at the locations Go's `os.UserCacheDir` uses so
/// both implementations agree on where the database lives.
fn user_cache_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LocalAppData")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| Error::external("%LocalAppData% is not defined"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(crate::config::home_dir()?.join("Library").join("Caches"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
            && xdg.is_absolute()
        {
            return Ok(xdg);
        }
        Ok(crate::config::home_dir()?.join(".cache"))
    }
}

impl Store {
    /// Opens the cache at [`path`].
    pub fn open_default() -> Result<Store> {
        Store::open(&path()?)
    }

    /// Opens or creates a cache database at `path`.
    pub fn open(path: &Path) -> Result<Store> {
        if path.as_os_str().is_empty() {
            return Err(msg!("caminho vazio para o cache"));
        }
        if let Some(dir) = path.parent() {
            create_private_dir(dir)
                .map_err(|err| msg!("criando diretório do cache: %w", Error::from(err)))?;
        }

        let mut conn =
            Connection::open(path).map_err(|err| msg!("abrindo cache: %w", Error::from(err)))?;
        initialize(&mut conn)?;
        protect_file(path)
            .map_err(|err| msg!("protegendo arquivo do cache: %w", Error::from(err)))?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// The last successful snapshot for `instance`. A changed provider or host
    /// invalidates data previously stored under the same instance name.
    pub fn load(&self, instance: &Instance) -> Result<Snapshot> {
        let name = &instance.name;
        let conn = self.conn();
        let synced_at: Option<i64> = conn
            .query_row(
                "SELECT synced_at FROM snapshots WHERE instance = ? AND provider = ? AND host = ?",
                params![name, instance.provider.as_str(), instance.host],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| msg!("lendo snapshot de %s: %w", name, Error::from(err)))?;
        let Some(synced_at) = synced_at else {
            return Ok(Snapshot::default());
        };

        let read_items =
            |err: rusqlite::Error| msg!("lendo itens de %s: %w", name, Error::from(err));
        let mut statement = conn
            .prepare("SELECT payload FROM items WHERE instance = ? ORDER BY item_key")
            .map_err(read_items)?;
        let mut rows = statement.query(params![name]).map_err(read_items)?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().map_err(read_items)? {
            let payload: Vec<u8> = row
                .get(0)
                .map_err(|err| msg!("lendo item de %s: %w", name, Error::from(err)))?;
            let item = serde_json::from_slice(&payload)
                .map_err(|err| msg!("decodificando item de %s: %w", name, Error::from(err)))?;
            items.push(item);
        }
        Ok(Snapshot {
            items,
            synced_at: Some(DateTime::from_timestamp_nanos(synced_at)),
        })
    }

    /// Atomically replaces one instance with a complete successful snapshot. A
    /// failed write leaves the previous snapshot untouched.
    pub fn replace(
        &self,
        instance: &Instance,
        items: &[Item],
        synced_at: DateTime<Utc>,
    ) -> Result<()> {
        let name = &instance.name;
        let mut encoded = Vec::with_capacity(items.len());
        for item in items {
            let key = item.key();
            let payload = serde_json::to_vec(item)
                .map_err(|err| msg!("codificando %s: %w", &key, Error::from(err)))?;
            encoded.push((key, payload));
        }
        // SQLite stores the instant as a 64-bit nanosecond count, which only
        // reaches the year 2262.
        let synced_at = synced_at.timestamp_nanos_opt().ok_or_else(|| {
            msg!(
                "salvando snapshot de %s: %w",
                name,
                Error::external("timestamp out of range")
            )
        })?;

        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| msg!("iniciando snapshot de %s: %w", name, Error::from(err)))?;
        tx.execute(
            "INSERT INTO snapshots(instance, provider, host, synced_at) VALUES (?, ?, ?, ?)
            ON CONFLICT(instance) DO UPDATE SET
                provider = excluded.provider,
                host = excluded.host,
                synced_at = excluded.synced_at",
            params![name, instance.provider.as_str(), instance.host, synced_at],
        )
        .map_err(|err| msg!("salvando snapshot de %s: %w", name, Error::from(err)))?;
        tx.execute("DELETE FROM items WHERE instance = ?", params![name])
            .map_err(|err| msg!("limpando snapshot de %s: %w", name, Error::from(err)))?;
        {
            let mut insert = tx
                .prepare("INSERT INTO items(instance, item_key, payload) VALUES (?, ?, ?)")
                .map_err(|err| msg!("preparando snapshot de %s: %w", name, Error::from(err)))?;
            for (key, payload) in &encoded {
                insert
                    .execute(params![name, key, payload])
                    .map_err(|err| msg!("salvando item %s: %w", key, Error::from(err)))?;
            }
        }
        tx.commit()
            .map_err(|err| msg!("confirmando snapshot de %s: %w", name, Error::from(err)))
    }

    /// A panic while the lock was held leaves no partial state behind: an open
    /// transaction rolls back when it is dropped, so the connection stays usable.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn initialize(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000;
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;",
    )
    .map_err(|err| msg!("configurando cache: %w", Error::from(err)))?;

    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|err| msg!("lendo versão do cache: %w", Error::from(err)))?;
    if version > SCHEMA_VERSION {
        return Err(msg!(
            "cache usa schema %d, mas esta versão suporta até %d",
            version,
            SCHEMA_VERSION
        ));
    }
    if version == SCHEMA_VERSION {
        return Ok(());
    }

    // Snapshots written by an older schema are discarded with their tables;
    // the next refresh rebuilds them.
    let tx = conn
        .transaction()
        .map_err(|err| msg!("iniciando schema do cache: %w", Error::from(err)))?;
    tx.execute_batch(&format!(
        "DROP TABLE IF EXISTS items;
        DROP TABLE IF EXISTS snapshots;
        CREATE TABLE snapshots (
            instance TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            host TEXT NOT NULL,
            synced_at INTEGER NOT NULL
        );
        CREATE TABLE items (
            instance TEXT NOT NULL REFERENCES snapshots(instance) ON DELETE CASCADE,
            item_key TEXT NOT NULL,
            payload BLOB NOT NULL,
            PRIMARY KEY (instance, item_key)
        );
        PRAGMA user_version = {SCHEMA_VERSION};"
    ))
    .map_err(|err| msg!("criando schema do cache: %w", Error::from(err)))?;
    tx.commit()
        .map_err(|err| msg!("salvando schema do cache: %w", Error::from(err)))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Windows has no owner-only mode bits; access control there comes from the
/// ACL inherited from the user's cache directory.
fn protect_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Provider;
    use crate::i18n::tests::env_lock;
    use crate::provider::{Kind, Relation};
    use chrono::{Duration, TimeZone};
    use std::ffi::OsString;

    /// Sets environment variables and restores the previous values on drop.
    struct Env(Vec<(&'static str, Option<OsString>)>);

    impl Env {
        fn set(&mut self, key: &'static str, value: impl AsRef<std::ffi::OsStr>) {
            self.0.push((key, std::env::var_os(key)));
            // SAFETY: every test that reads or writes the environment holds env_lock.
            unsafe { std::env::set_var(key, value) }
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            for (key, previous) in self.0.drain(..).rev() {
                // SAFETY: the test that owns this guard still holds env_lock.
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn instance(name: &str, provider: Provider, host: &str) -> Instance {
        Instance {
            name: name.into(),
            provider,
            host: host.into(),
            ..Instance::default()
        }
    }

    fn seconds(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn store_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Store>();
    }

    #[test]
    fn path_override() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let want = dir.path().join("custom.db");
        let mut env = Env(Vec::new());
        env.set(PATH_ENV, &want);
        assert_eq!(path().unwrap(), want);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn default_path_follows_the_user_cache_dir() {
        let _guard = env_lock();
        let mut env = Env(Vec::new());
        env.set(PATH_ENV, "");
        env.set("HOME", "/home/u");
        env.set("XDG_CACHE_HOME", "/xdg");
        assert_eq!(path().unwrap(), PathBuf::from("/xdg/pr-tracker/cache.db"));
        for not_absolute in ["", "relative"] {
            env.set("XDG_CACHE_HOME", not_absolute);
            assert_eq!(
                path().unwrap(),
                PathBuf::from("/home/u/.cache/pr-tracker/cache.db")
            );
        }
    }

    #[test]
    fn replace_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cache.db");
        let store = Store::open(&path).unwrap();

        let instance = instance("work", Provider::GitHub, "github.example.com");
        let synced_at =
            Utc.with_ymd_and_hms(2026, 9, 19, 12, 30, 0).unwrap() + Duration::nanoseconds(123);
        let items = vec![Item {
            kind: Kind::Pr,
            instance: instance.name.clone(),
            provider: instance.provider,
            host: instance.host.clone(),
            repo: "acme/app".into(),
            number: 42,
            title: "Ship cache".into(),
            relations: Relation::AUTHORED | Relation::ASSIGNED,
            updated_at: Some(synced_at - Duration::minutes(1)),
            labels: vec!["ready".into()],
            ..Item::default()
        }];
        store.replace(&instance, &items, synced_at).unwrap();

        let got = store.load(&instance).unwrap();
        assert_eq!(got.synced_at, Some(synced_at));
        assert_eq!(got.items, items);
        // Windows has no POSIX permission bits and uses ACLs for access
        // control, so an exact 0600 assertion is meaningful only on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "cache mode = {mode:o}, want 600");
        }
    }

    #[test]
    fn replace_is_atomic_and_clears_old_items() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cache.db")).unwrap();
        let instance = instance("github.com", Provider::GitHub, "github.com");
        let item = |number, title: &str| Item {
            instance: instance.name.clone(),
            repo: "o/r".into(),
            number,
            title: title.into(),
            ..Item::default()
        };
        store
            .replace(&instance, &[item(1, "old")], seconds(1))
            .unwrap();

        let duplicate = [item(2, "first"), item(2, "duplicate")];
        assert!(
            store.replace(&instance, &duplicate, seconds(2)).is_err(),
            "expected duplicate key error"
        );
        let got = store.load(&instance).unwrap();
        assert_eq!(
            got,
            Snapshot {
                items: vec![item(1, "old")],
                synced_at: Some(seconds(1))
            },
            "failed replacement changed snapshot"
        );

        store.replace(&instance, &[], seconds(3)).unwrap();
        let got = store.load(&instance).unwrap();
        assert_eq!(
            got,
            Snapshot {
                items: Vec::new(),
                synced_at: Some(seconds(3))
            },
            "empty replacement"
        );
    }

    #[test]
    fn load_rejects_changed_instance_scope() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cache.db")).unwrap();
        let mut instance = instance("work", Provider::GitLab, "gitlab.old");
        let item = Item {
            instance: instance.name.clone(),
            repo: "g/r".into(),
            number: 1,
            ..Item::default()
        };
        store.replace(&instance, &[item], Utc::now()).unwrap();

        instance.host = "gitlab.new".into();
        assert_eq!(
            store.load(&instance).unwrap(),
            Snapshot::default(),
            "changed host loaded stale data"
        );
        instance.host = "gitlab.old".into();
        instance.provider = Provider::Gitea;
        assert_eq!(
            store.load(&instance).unwrap(),
            Snapshot::default(),
            "changed provider loaded stale data"
        );
    }

    #[test]
    fn undecodable_item_fails_the_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cache.db")).unwrap();
        let instance = instance("work", Provider::GitHub, "github.com");
        store.replace(&instance, &[], seconds(1)).unwrap();
        store.conn().execute("INSERT INTO items(instance, item_key, payload) VALUES ('work', 'k', CAST('not json' AS BLOB))", []).unwrap();
        let err = store.load(&instance).unwrap_err();
        assert!(err.is("decodificando item de %s: %w"), "{err}");
    }

    #[test]
    fn version_1_database_is_emptied_and_upgraded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"CREATE TABLE snapshots (
                    instance TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    host TEXT NOT NULL,
                    synced_at INTEGER NOT NULL
                );
                CREATE TABLE items (
                    instance TEXT NOT NULL REFERENCES snapshots(instance) ON DELETE CASCADE,
                    item_key TEXT NOT NULL,
                    payload BLOB NOT NULL,
                    PRIMARY KEY (instance, item_key)
                );
                INSERT INTO snapshots VALUES ('work', 'github', 'github.com', 1000000000);
                INSERT INTO items VALUES ('work', 'work|o/r|0#1', CAST('{"Kind":0,"Instance":"work","Repo":"o/r","Number":1,"Title":"go"}' AS BLOB));
                PRAGMA user_version = 1;"#,
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let instance = instance("work", Provider::GitHub, "github.com");
        assert_eq!(
            store.load(&instance).unwrap(),
            Snapshot::default(),
            "version 1 snapshot survived the migration"
        );
        let version: i64 = store
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let items = vec![Item {
            instance: "work".into(),
            repo: "o/r".into(),
            number: 1,
            title: "rust".into(),
            ..Item::default()
        }];
        store.replace(&instance, &items, seconds(5)).unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened.load(&instance).unwrap(),
            Snapshot {
                items,
                synced_at: Some(seconds(5))
            }
        );
    }

    #[test]
    fn newer_schema_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1))
            .unwrap();
        let err = Store::open(&path).err().expect("newer schema accepted");
        assert!(
            err.is("cache usa schema %d, mas esta versão suporta até %d"),
            "{err}"
        );
    }
}
