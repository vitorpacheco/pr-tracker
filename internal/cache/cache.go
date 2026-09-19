// Package cache persists disposable snapshots of provider items in SQLite.
// The remote providers remain the source of truth; a cache can always be
// deleted and rebuilt by the next successful refresh.
package cache

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	_ "modernc.org/sqlite"
)

const (
	appName       = "pr-tracker"
	pathEnv       = "PR_TRACKER_CACHE"
	schemaVersion = 1
)

// Snapshot is the last complete, successful refresh of one instance.
type Snapshot struct {
	Items    []provider.Item
	SyncedAt time.Time
}

// Store owns the SQLite cache. It is safe for concurrent callers.
type Store struct {
	db *sql.DB
}

// Path returns the cache database path. PR_TRACKER_CACHE overrides it.
func Path() (string, error) {
	if path := os.Getenv(pathEnv); path != "" {
		return path, nil
	}
	dir, err := os.UserCacheDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, appName, "cache.db"), nil
}

// OpenDefault opens the cache at Path.
func OpenDefault() (*Store, error) {
	path, err := Path()
	if err != nil {
		return nil, err
	}
	return Open(path)
}

// Open opens or creates a cache database at path.
func Open(path string) (*Store, error) {
	if path == "" {
		return nil, errors.New("caminho vazio para o cache")
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("criando diretório do cache: %w", err)
	}

	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("abrindo cache: %w", err)
	}
	// One connection is enough for this small local cache and makes per-
	// connection SQLite pragmas deterministic. WAL still permits another
	// pr-tracker process to read while this one commits a snapshot.
	db.SetMaxOpenConns(1)
	store := &Store{db: db}
	if err := store.initialize(); err != nil {
		db.Close()
		return nil, err
	}
	// Windows only uses the owner-write bit here; access control continues to
	// come from the ACL inherited from the user's cache directory.
	if err := os.Chmod(path, 0o600); err != nil {
		db.Close()
		return nil, fmt.Errorf("protegendo arquivo do cache: %w", err)
	}
	return store, nil
}

func (s *Store) initialize() error {
	for _, pragma := range []string{
		"PRAGMA busy_timeout = 5000",
		"PRAGMA foreign_keys = ON",
		"PRAGMA journal_mode = WAL",
	} {
		if _, err := s.db.Exec(pragma); err != nil {
			return fmt.Errorf("configurando cache: %w", err)
		}
	}

	var version int
	if err := s.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		return fmt.Errorf("lendo versão do cache: %w", err)
	}
	if version > schemaVersion {
		return fmt.Errorf("cache usa schema %d, mas esta versão suporta até %d", version, schemaVersion)
	}
	if version == schemaVersion {
		return nil
	}

	tx, err := s.db.Begin()
	if err != nil {
		return fmt.Errorf("iniciando schema do cache: %w", err)
	}
	defer tx.Rollback()
	statements := []string{
		`CREATE TABLE snapshots (
			instance TEXT PRIMARY KEY,
			provider TEXT NOT NULL,
			host TEXT NOT NULL,
			synced_at INTEGER NOT NULL
		)`,
		`CREATE TABLE items (
			instance TEXT NOT NULL REFERENCES snapshots(instance) ON DELETE CASCADE,
			item_key TEXT NOT NULL,
			payload BLOB NOT NULL,
			PRIMARY KEY (instance, item_key)
		)`,
		fmt.Sprintf("PRAGMA user_version = %d", schemaVersion),
	}
	for _, statement := range statements {
		if _, err := tx.Exec(statement); err != nil {
			return fmt.Errorf("criando schema do cache: %w", err)
		}
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("salvando schema do cache: %w", err)
	}
	return nil
}

// Load returns the last successful snapshot for in. A changed provider or
// host invalidates data previously stored under the same instance name.
func (s *Store) Load(ctx context.Context, in config.Instance) (Snapshot, error) {
	var syncedAt int64
	err := s.db.QueryRowContext(ctx,
		`SELECT synced_at FROM snapshots WHERE instance = ? AND provider = ? AND host = ?`,
		in.Name, in.Provider, in.Host,
	).Scan(&syncedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return Snapshot{}, nil
	}
	if err != nil {
		return Snapshot{}, fmt.Errorf("lendo snapshot de %s: %w", in.Name, err)
	}

	rows, err := s.db.QueryContext(ctx,
		`SELECT payload FROM items WHERE instance = ? ORDER BY item_key`, in.Name)
	if err != nil {
		return Snapshot{}, fmt.Errorf("lendo itens de %s: %w", in.Name, err)
	}
	defer rows.Close()

	snapshot := Snapshot{SyncedAt: time.Unix(0, syncedAt)}
	for rows.Next() {
		var payload []byte
		if err := rows.Scan(&payload); err != nil {
			return Snapshot{}, fmt.Errorf("lendo item de %s: %w", in.Name, err)
		}
		var item provider.Item
		if err := json.Unmarshal(payload, &item); err != nil {
			return Snapshot{}, fmt.Errorf("decodificando item de %s: %w", in.Name, err)
		}
		snapshot.Items = append(snapshot.Items, item)
	}
	if err := rows.Err(); err != nil {
		return Snapshot{}, fmt.Errorf("lendo itens de %s: %w", in.Name, err)
	}
	return snapshot, nil
}

// Replace atomically replaces one instance with a complete successful
// snapshot. A failed write leaves the previous snapshot untouched.
func (s *Store) Replace(ctx context.Context, in config.Instance, items []provider.Item, syncedAt time.Time) error {
	type encodedItem struct {
		key     string
		payload []byte
	}
	encoded := make([]encodedItem, 0, len(items))
	for i := range items {
		payload, err := json.Marshal(&items[i])
		if err != nil {
			return fmt.Errorf("codificando %s: %w", items[i].Key(), err)
		}
		encoded = append(encoded, encodedItem{key: items[i].Key(), payload: payload})
	}

	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("iniciando snapshot de %s: %w", in.Name, err)
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO snapshots(instance, provider, host, synced_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(instance) DO UPDATE SET
			provider = excluded.provider,
			host = excluded.host,
			synced_at = excluded.synced_at`,
		in.Name, in.Provider, in.Host, syncedAt.UnixNano()); err != nil {
		return fmt.Errorf("salvando snapshot de %s: %w", in.Name, err)
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM items WHERE instance = ?`, in.Name); err != nil {
		return fmt.Errorf("limpando snapshot de %s: %w", in.Name, err)
	}
	stmt, err := tx.PrepareContext(ctx, `INSERT INTO items(instance, item_key, payload) VALUES (?, ?, ?)`)
	if err != nil {
		return fmt.Errorf("preparando snapshot de %s: %w", in.Name, err)
	}
	defer stmt.Close()
	for _, item := range encoded {
		if _, err := stmt.ExecContext(ctx, in.Name, item.key, item.payload); err != nil {
			return fmt.Errorf("salvando item %s: %w", item.key, err)
		}
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("confirmando snapshot de %s: %w", in.Name, err)
	}
	return nil
}

// Close closes the cache database.
func (s *Store) Close() error { return s.db.Close() }
