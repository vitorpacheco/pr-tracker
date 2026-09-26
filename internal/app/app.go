// Package app owns the orchestration shared by the terminal and desktop adapters.
package app

import (
	"context"
	"errors"
	"maps"
	"slices"
	"sort"
	"sync"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

type snapshotStore interface {
	Load(context.Context, config.Instance) (cache.Snapshot, error)
	Replace(context.Context, config.Instance, []provider.Item, time.Time) error
}

// Application captures configuration for one generation of background work.
// Recreate it after settings change; in-flight work keeps its original settings.
// The caller owns the cache's lifetime and operation deadlines.
type Application struct {
	cfg            config.Config
	store          snapshotStore
	client         func(config.Instance, ...config.Repo) provider.Client
	resolveClone   func(context.Context, *config.Config, *provider.Item) (string, string, bool)
	worktreeExists func(*config.Config, *provider.Item) (string, bool)
	createWorktree func(context.Context, *config.Config, provider.Client, *provider.Item, string, string) (string, error)
	removeWorktree func(context.Context, *config.Config, *provider.Item, bool) error
}

func New(cfg *config.Config, store *cache.Store) *Application {
	a := &Application{cfg: *cfg, client: provider.New, resolveClone: gitops.ResolveClone, worktreeExists: gitops.WorktreeExists, createWorktree: gitops.CreateWorktree, removeWorktree: gitops.RemoveWorktree}
	a.cfg.ToolPaths = maps.Clone(cfg.ToolPaths)
	a.cfg.Instances = slices.Clone(cfg.Instances)
	a.cfg.Repos = slices.Clone(cfg.Repos)
	a.cfg.CloneRoots = slices.Clone(cfg.CloneRoots)
	if store != nil {
		a.store = store
	}
	return a
}

// Cached contains only complete snapshots, including successful empty lists.
type Cached struct {
	Items    map[string][]provider.Item
	SyncedAt time.Time
}

// RefreshResult distinguishes remote failures from failures to persist fresh data.
// A missing instance in Items preserves its previous items; an empty entry clears them.
type RefreshResult struct {
	Items       map[string][]provider.Item
	Errors      map[string]error
	CacheErrors map[string]error
	Clones      map[string]string
}

// Snapshot is presentation-independent list state after applying a refresh.
type Snapshot struct {
	Items      []provider.Item
	Errors     map[string]error
	Clones     map[string]string
	CacheError error
}

func (a *Application) Load(ctx context.Context) (Cached, error) {
	result := Cached{Items: map[string][]provider.Item{}}
	if a.store == nil {
		return result, nil
	}
	var errs []error
	for _, in := range a.cfg.Instances {
		if in.Disabled {
			continue
		}
		snapshot, err := a.store.Load(ctx, in)
		if err != nil {
			errs = append(errs, err)
			continue
		}
		if snapshot.SyncedAt.IsZero() {
			continue
		}
		result.Items[in.Name] = snapshot.Items
		if snapshot.SyncedAt.After(result.SyncedAt) {
			result.SyncedAt = snapshot.SyncedAt
		}
	}
	return result, errors.Join(errs...)
}

func (a *Application) Refresh(ctx context.Context) RefreshResult {
	ctx = toolchain.WithPaths(ctx, a.cfg.ToolPaths)
	result := RefreshResult{Items: map[string][]provider.Item{}, Errors: map[string]error{}, CacheErrors: map[string]error{}, Clones: map[string]string{}}
	var mu sync.Mutex
	var wg sync.WaitGroup
	for _, in := range a.cfg.Instances {
		if in.Disabled {
			continue
		}
		client := a.client(in, a.cfg.Repos...)
		wg.Go(func() {
			items, err := client.List(ctx)
			var cacheErr error
			clones := map[string]string{}
			if err == nil {
				if a.store != nil {
					cacheErr = a.store.Replace(ctx, in, items, time.Now())
				}
				for i := range items {
					if path, _, ok := a.resolveClone(ctx, &a.cfg, &items[i]); ok {
						clones[items[i].Key()] = path
					}
				}
			}
			mu.Lock()
			defer mu.Unlock()
			if err != nil {
				result.Errors[in.Name] = err
				return
			}
			result.Items[in.Name] = items
			if cacheErr != nil {
				result.CacheErrors[in.Name] = cacheErr
			}
			for key, path := range clones {
				result.Clones[key] = path
			}
		})
	}
	wg.Wait()
	return result
}

// Reconcile preserves stale data on remote failure and drops removed or disabled
// instances. It does not mutate the previous snapshot or the refresh result.
func (a *Application) Reconcile(previous Snapshot, result RefreshResult) Snapshot {
	byInstance := map[string][]provider.Item{}
	for _, item := range previous.Items {
		byInstance[item.Instance] = append(byInstance[item.Instance], item)
	}
	next := Snapshot{Errors: map[string]error{}, Clones: map[string]string{}}
	var cacheErrs []error
	for _, in := range a.cfg.Instances {
		if in.Disabled {
			continue
		}
		items, ok := result.Items[in.Name]
		if !ok {
			items = byInstance[in.Name]
		}
		next.Items = append(next.Items, items...)
		if err := result.Errors[in.Name]; err != nil {
			next.Errors[in.Name] = err
		}
		if err := result.CacheErrors[in.Name]; err != nil {
			cacheErrs = append(cacheErrs, err)
		}
	}
	sort.SliceStable(next.Items, func(i, j int) bool { return next.Items[i].UpdatedAt.After(next.Items[j].UpdatedAt) })
	for _, item := range next.Items {
		key := item.Key()
		if path, ok := result.Clones[key]; ok {
			next.Clones[key] = path
		} else if result.Errors[item.Instance] != nil {
			if path, ok := previous.Clones[key]; ok {
				next.Clones[key] = path
			}
		}
	}
	next.CacheError = errors.Join(cacheErrs...)
	return next
}
