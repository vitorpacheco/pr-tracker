package app

import (
	"context"
	"errors"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

type fakeClient struct {
	provider.Client
	list func(context.Context) ([]provider.Item, error)
}

func (c fakeClient) List(ctx context.Context) ([]provider.Item, error) { return c.list(ctx) }

type failingStore struct{ snapshotStore }

func (failingStore) Replace(context.Context, config.Instance, []provider.Item, time.Time) error {
	return errors.New("disk full")
}

func TestLoadAndRefreshPreserveFailedInstance(t *testing.T) {
	ctx := context.Background()
	cfg := config.Default()
	cfg.Instances = []config.Instance{
		{Name: "offline", Provider: config.GitHub, Host: "github.com"},
		{Name: "online", Provider: config.GitLab, Host: "gitlab.com"},
		{Name: "disabled", Disabled: true},
	}
	store, err := cache.Open(filepath.Join(t.TempDir(), "cache.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	syncedAt := time.Date(2026, 9, 1, 0, 0, 0, 0, time.UTC)
	for _, in := range cfg.Instances {
		if err := store.Replace(ctx, in, []provider.Item{{Instance: in.Name, Number: 1}}, syncedAt); err != nil {
			t.Fatal(err)
		}
	}
	a := New(cfg, store)
	a.client = func(in config.Instance, _ ...config.Repo) provider.Client {
		if in.Disabled {
			t.Error("disabled provider constructed")
		}
		return fakeClient{list: func(context.Context) ([]provider.Item, error) {
			if in.Name == "offline" {
				return []provider.Item{{Instance: in.Name, Number: 99}}, errors.New("offline")
			}
			return []provider.Item{{Instance: in.Name, Number: 2, UpdatedAt: syncedAt}}, nil
		}}
	}
	a.resolveClone = func(_ context.Context, _ *config.Config, item *provider.Item) (string, string, bool) {
		if item.Instance == "offline" {
			t.Error("resolved clone for incomplete provider result")
		}
		return "/clone", "origin", true
	}
	loaded, err := a.Load(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Items) != 2 || !loaded.SyncedAt.Equal(syncedAt) {
		t.Fatalf("cache = %+v", loaded)
	}
	previous := a.Reconcile(Snapshot{}, RefreshResult{Items: loaded.Items})
	staleKey := loaded.Items["offline"][0].Key()
	previous.Clones[staleKey] = "/old-clone"
	result := a.Refresh(ctx)
	next := a.Reconcile(previous, result)
	if len(next.Items) != 2 || next.Items[0].Number != 2 || next.Items[1].Number != 1 {
		t.Fatalf("items = %+v", next.Items)
	}
	if next.Errors["offline"] == nil || next.Clones[staleKey] != "/old-clone" {
		t.Fatalf("lost offline state: %+v", next)
	}
	persisted, err := store.Load(ctx, cfg.Instances[0])
	if err != nil || persisted.Items[0].Number != 1 || !persisted.SyncedAt.Equal(syncedAt) {
		t.Fatalf("failed refresh replaced cache: %+v, %v", persisted, err)
	}
	persisted, err = store.Load(ctx, cfg.Instances[1])
	if err != nil || len(persisted.Items) != 1 || persisted.Items[0].Number != 2 {
		t.Fatalf("fresh cache = %+v, %v", persisted, err)
	}
}

func TestRefreshRunsInstancesConcurrentlyAndPropagatesCancellation(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "one"}, {Name: "two"}}
	a := New(cfg, nil)
	started := make(chan struct{}, 2)
	a.client = func(config.Instance, ...config.Repo) provider.Client {
		return fakeClient{list: func(ctx context.Context) ([]provider.Item, error) {
			started <- struct{}{}
			<-ctx.Done()
			return nil, ctx.Err()
		}}
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan RefreshResult, 1)
	go func() { done <- a.Refresh(ctx) }()
	timeout := time.NewTimer(5 * time.Second)
	defer timeout.Stop()
	for range 2 {
		select {
		case <-started:
		case <-timeout.C:
			t.Fatal("instances did not start concurrently")
		}
	}
	cancel()
	select {
	case result := <-done:
		if len(result.Items) != 0 || len(result.Errors) != 2 {
			t.Fatalf("result = %+v", result)
		}
		for _, err := range result.Errors {
			if !errors.Is(err, context.Canceled) {
				t.Fatal(err)
			}
		}
	case <-timeout.C:
		t.Fatal("refresh ignored cancellation")
	}
}

func TestRefreshCacheWriteFailureKeepsFreshItems(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "one"}}
	a := New(cfg, nil)
	a.store = failingStore{}
	a.client = func(config.Instance, ...config.Repo) provider.Client {
		return fakeClient{list: func(context.Context) ([]provider.Item, error) {
			return []provider.Item{{Instance: "one", Number: 2}}, nil
		}}
	}
	a.resolveClone = func(context.Context, *config.Config, *provider.Item) (string, string, bool) { return "", "", false }
	result := a.Refresh(context.Background())
	next := a.Reconcile(Snapshot{}, result)
	if len(next.Items) != 1 || next.Items[0].Number != 2 || len(next.Errors) != 0 || next.CacheError == nil {
		t.Fatalf("snapshot = %+v", next)
	}
}

func TestReconcileClearsEmptyDisabledAndRemovedInstances(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "empty"}, {Name: "disabled", Disabled: true}, {Name: "failed"}}
	a := New(cfg, nil)
	previous := Snapshot{Clones: map[string]string{}}
	for _, name := range []string{"empty", "disabled", "removed", "failed"} {
		item := provider.Item{Instance: name, Number: 1}
		previous.Items = append(previous.Items, item)
		previous.Clones[item.Key()] = "/" + name
	}
	result := RefreshResult{Items: map[string][]provider.Item{"empty": nil, "disabled": {{Instance: "disabled"}}}, Errors: map[string]error{"failed": errors.New("offline")}}
	next := a.Reconcile(previous, result)
	if len(next.Items) != 1 || next.Items[0].Instance != "failed" || len(next.Clones) != 1 {
		t.Fatalf("snapshot = %+v", next)
	}
	if len(previous.Items) != 4 || len(previous.Clones) != 4 || result.Clones != nil {
		t.Fatal("reconciliation mutated its inputs")
	}
}

func TestApplicationCapturesConfiguration(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "original"}}
	cfg.Repos = []config.Repo{{Instance: "original", Name: "org/repo", TrackAll: true}}
	cfg.CloneRoots = []string{"/original"}
	a := New(cfg, nil)
	cfg.Instances[0].Name = "edited"
	cfg.Repos[0].TrackAll = false
	cfg.CloneRoots[0] = "/edited"
	var calls sync.Map
	a.client = func(in config.Instance, repos ...config.Repo) provider.Client {
		if in.Name != "original" || !repos[0].TrackAll {
			t.Errorf("configuration changed: %+v, %+v", in, repos)
		}
		return fakeClient{list: func(context.Context) ([]provider.Item, error) { calls.Store(in.Name, true); return nil, nil }}
	}
	if a.cfg.CloneRoots[0] != "/original" {
		t.Fatal("clone roots alias mutable configuration")
	}
	loaded, err := a.Load(context.Background())
	if err != nil || len(loaded.Items) != 0 {
		t.Fatalf("nil cache load = %+v, %v", loaded, err)
	}
	result := a.Refresh(context.Background())
	if _, ok := result.Items["original"]; !ok {
		t.Fatalf("missing empty successful result: %+v", result)
	}
	if _, ok := calls.Load("edited"); ok {
		t.Fatal("queried edited configuration")
	}
}

type partialStore struct{ snapshotStore }

func (partialStore) Load(_ context.Context, in config.Instance) (cache.Snapshot, error) {
	switch in.Name {
	case "broken":
		return cache.Snapshot{}, errors.New("invalid cached payload")
	case "missing":
		return cache.Snapshot{}, nil
	default:
		return cache.Snapshot{SyncedAt: time.Unix(100, 0)}, nil
	}
}

func TestLoadReturnsUsableSnapshotsAlongsideCacheErrors(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "broken"}, {Name: "missing"}, {Name: "empty"}}
	a := New(cfg, nil)
	a.store = partialStore{}
	loaded, err := a.Load(context.Background())
	if err == nil {
		t.Fatal("cache read failure was hidden")
	}
	if len(loaded.Items) != 1 {
		t.Fatalf("snapshots = %+v", loaded.Items)
	}
	if _, ok := loaded.Items["empty"]; !ok {
		t.Fatal("successful empty snapshot was discarded")
	}
	if !loaded.SyncedAt.Equal(time.Unix(100, 0)) {
		t.Fatalf("sync time = %v", loaded.SyncedAt)
	}
}
