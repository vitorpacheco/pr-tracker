package cache

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

func TestPathOverride(t *testing.T) {
	want := filepath.Join(t.TempDir(), "custom.db")
	t.Setenv("PR_TRACKER_CACHE", want)
	got, err := Path()
	if err != nil || got != want {
		t.Fatalf("Path() = %q, %v; want %q", got, err, want)
	}
}

func TestReplaceLoadRoundTrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "cache.db")
	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })

	in := config.Instance{Name: "work", Provider: config.GitHub, Host: "github.example.com"}
	syncedAt := time.Date(2026, 9, 19, 12, 30, 0, 123, time.UTC)
	items := []provider.Item{{
		Kind: provider.KindPR, Instance: in.Name, Provider: in.Provider, Host: in.Host,
		Repo: "acme/app", Number: 42, Title: "Ship cache", Relations: provider.Authored | provider.Assigned,
		UpdatedAt: syncedAt.Add(-time.Minute), Labels: []string{"ready"},
	}}
	if err := store.Replace(t.Context(), in, items, syncedAt); err != nil {
		t.Fatal(err)
	}

	got, err := store.Load(t.Context(), in)
	if err != nil {
		t.Fatal(err)
	}
	if !got.SyncedAt.Equal(syncedAt) || len(got.Items) != 1 {
		t.Fatalf("snapshot = %+v", got)
	}
	if got.Items[0].Title != "Ship cache" || got.Items[0].Relations != provider.Authored|provider.Assigned || len(got.Items[0].Labels) != 1 {
		t.Fatalf("item = %+v", got.Items[0])
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	// Windows synthesizes POSIX permission bits as 0666 and uses ACLs for
	// access control, so an exact 0600 assertion is meaningful only on Unix.
	if runtime.GOOS != "windows" && info.Mode().Perm() != 0o600 {
		t.Fatalf("cache mode = %o, want 600", info.Mode().Perm())
	}
}

func TestReplaceIsAtomicAndClearsOldItems(t *testing.T) {
	store, err := Open(filepath.Join(t.TempDir(), "cache.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	in := config.Instance{Name: "github.com", Provider: config.GitHub, Host: "github.com"}
	old := []provider.Item{{Instance: in.Name, Repo: "o/r", Number: 1, Title: "old"}}
	if err := store.Replace(t.Context(), in, old, time.Unix(1, 0)); err != nil {
		t.Fatal(err)
	}

	duplicate := []provider.Item{
		{Instance: in.Name, Repo: "o/r", Number: 2, Title: "first"},
		{Instance: in.Name, Repo: "o/r", Number: 2, Title: "duplicate"},
	}
	if err := store.Replace(t.Context(), in, duplicate, time.Unix(2, 0)); err == nil {
		t.Fatal("expected duplicate key error")
	}
	got, err := store.Load(t.Context(), in)
	if err != nil || len(got.Items) != 1 || got.Items[0].Title != "old" || !got.SyncedAt.Equal(time.Unix(1, 0)) {
		t.Fatalf("failed replacement changed snapshot: %+v, %v", got, err)
	}

	if err := store.Replace(t.Context(), in, nil, time.Unix(3, 0)); err != nil {
		t.Fatal(err)
	}
	got, err = store.Load(t.Context(), in)
	if err != nil || len(got.Items) != 0 || !got.SyncedAt.Equal(time.Unix(3, 0)) {
		t.Fatalf("empty replacement = %+v, %v", got, err)
	}
}

func TestLoadRejectsChangedInstanceScope(t *testing.T) {
	store, err := Open(filepath.Join(t.TempDir(), "cache.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	in := config.Instance{Name: "work", Provider: config.GitLab, Host: "gitlab.old"}
	if err := store.Replace(t.Context(), in, []provider.Item{{Instance: in.Name, Repo: "g/r", Number: 1}}, time.Now()); err != nil {
		t.Fatal(err)
	}
	in.Host = "gitlab.new"
	got, err := store.Load(t.Context(), in)
	if err != nil || !got.SyncedAt.IsZero() || len(got.Items) != 0 {
		t.Fatalf("changed scope loaded stale data: %+v, %v", got, err)
	}
}
