package app

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

func TestSessionSharesRefreshAndReturnsDetachedState(t *testing.T) {
	a, _, item := actionFixture()
	item.Labels = []string{"original"}
	s := NewSession(&a.cfg, nil)
	s.application = a
	s.loaded = true
	defer s.Close()
	started := make(chan struct{})
	release := make(chan struct{})
	var calls atomic.Int32
	a.client = func(config.Instance, ...config.Repo) provider.Client {
		return fakeClient{list: func(ctx context.Context) ([]provider.Item, error) {
			calls.Add(1)
			close(started)
			select {
			case <-release:
				return []provider.Item{item}, nil
			case <-ctx.Done():
				return nil, ctx.Err()
			}
		}}
	}
	result := make(chan State, 1)
	go func() { state, _ := s.Refresh(context.Background()); result <- state }()
	<-started
	// A cancelled waiter must not cancel the underlying shared refresh.
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := s.Refresh(ctx); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	close(release)
	state := <-result
	if calls.Load() != 1 || len(state.Items) != 1 {
		t.Fatalf("calls = %d, state = %+v", calls.Load(), state)
	}
	state.Items[0].Labels[0] = "mutated"
	state.Clones[item.Key()] = "mutated"
	loaded, err := s.Load(context.Background())
	if err != nil || loaded.Items[0].Labels[0] != "original" || loaded.Clones[item.Key()] == "mutated" {
		t.Fatalf("state escaped: %+v, %v", loaded, err)
	}
}

func TestSessionDiscardsOldGenerationAndClosesWork(t *testing.T) {
	a, _, item := actionFixture()
	s := NewSession(&a.cfg, nil)
	s.application = a
	s.loaded = true
	defer s.Close()
	started := make(chan struct{})
	release := make(chan struct{})
	a.client = func(config.Instance, ...config.Repo) provider.Client {
		return fakeClient{list: func(ctx context.Context) ([]provider.Item, error) {
			close(started)
			select {
			case <-release:
				return []provider.Item{item}, nil
			case <-ctx.Done():
				return nil, ctx.Err()
			}
		}}
	}
	done := make(chan State, 1)
	go func() { state, _ := s.Refresh(context.Background()); done <- state }()
	<-started
	cfg := s.Configuration()
	cfg.Instances = nil
	s.Configure(&cfg)
	close(release)
	select {
	case state := <-done:
		if len(state.Items) != 0 || state.Revision != 1 {
			t.Fatalf("old result accepted: %+v", state)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("refresh did not follow the new generation")
	}
	s.Close()
	if _, err := s.Refresh(context.Background()); !errors.Is(err, ErrClosed) {
		t.Fatal(err)
	}
}

func TestSessionResolvesCommandItemAndSerializesItemActions(t *testing.T) {
	a, c, item := actionFixture()
	a.cfg.Language = "pt"
	s := NewSession(&a.cfg, nil)
	s.application = a
	s.loaded = true
	s.state.Items = []provider.Item{item}
	defer s.Close()
	spoofed := item
	spoofed.Repo = "other/repo"
	out, err := s.Execute(context.Background(), item.Key(), Command{Action: Comment, Item: spoofed, Body: "hello"})
	if err != nil || out.Message != "comentário enviado em org/repo#7" || c.body != "hello" {
		t.Fatalf("command item escaped state lookup: %+v, %v", out, err)
	}
	s.busy[item.Key()] = true
	if _, err := s.Execute(context.Background(), item.Key(), Command{Action: Approve}); !errors.Is(err, ErrBusy) {
		t.Fatal(err)
	}
	if _, err := s.Detail(context.Background(), "missing"); err == nil {
		t.Fatal("unknown item accepted")
	}
}

func TestSaveSettingsRollbackAndIsolation(t *testing.T) {
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	cfg := config.Default()
	s := NewSession(cfg, nil)
	defer s.Close()
	next := s.Configuration()
	next.Instances = []config.Instance{{Provider: config.GitHub}}
	if err := s.SaveSettings(next); err != nil {
		t.Fatal(err)
	}
	next.Instances[0].Name = "mutated"
	saved := s.Configuration()
	if saved.Instances[0].Name != "github.com" {
		t.Fatalf("settings = %+v", saved)
	}
	next = s.Configuration()
	next.Terminal = "invalid"
	if err := s.SaveSettings(next); err == nil {
		t.Fatal("invalid settings accepted")
	}
	if s.Configuration().Terminal != "auto" {
		t.Fatal("failed settings changed live config")
	}
	persisted, _, err := config.Load()
	if err != nil || persisted.Terminal != "auto" {
		t.Fatalf("persisted = %+v, %v", persisted, err)
	}
}

func TestDirtyCleanupRemainsPossibleAfterClosedItemDisappears(t *testing.T) {
	a, _, item := actionFixture()
	s := NewSession(&a.cfg, nil)
	s.application = a
	s.loaded = true
	s.state.Items = []provider.Item{item}
	defer s.Close()
	a.removeWorktree = func(_ context.Context, _ *config.Config, _ *provider.Item, force bool) error {
		if force {
			return nil
		}
		return ErrDirtyWorktree
	}
	out, err := s.Execute(context.Background(), item.Key(), Command{Action: Close, RemoveAfter: true})
	if !errors.Is(err, ErrDirtyWorktree) || !out.Refresh {
		t.Fatalf("close = %+v, %v", out, err)
	}
	// Closed PRs are absent from the next remote listing.
	s.state.Items = nil
	if _, err := s.Execute(context.Background(), item.Key(), Command{Action: RemoveWorktree, Force: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Execute(context.Background(), item.Key(), Command{Action: RemoveWorktree, Force: true}); err == nil {
		t.Fatal("completed cleanup remained authorized")
	}
}

func TestSaveSettingsWriteFailureLeavesLiveConfigUnchanged(t *testing.T) {
	parent := filepath.Join(t.TempDir(), "not-a-directory")
	if err := os.WriteFile(parent, []byte("keep"), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(parent, "config.toml"))
	s := NewSession(config.Default(), nil)
	defer s.Close()
	cfg := s.Configuration()
	cfg.RefreshInterval = "1m"
	if err := s.SaveSettings(cfg); err == nil {
		t.Fatal("write to file parent unexpectedly succeeded")
	}
	if s.Configuration().RefreshInterval != "5m" || s.revision != 0 {
		t.Fatal("failed write changed the live generation")
	}
}

func TestSessionCoalescesWaitersAndCancelsOnClose(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a, _, item := actionFixture()
		s := NewSession(&a.cfg, nil)
		s.application = a
		s.loaded = true
		release := make(chan struct{})
		var calls atomic.Int32
		a.client = func(config.Instance, ...config.Repo) provider.Client {
			return fakeClient{list: func(ctx context.Context) ([]provider.Item, error) {
				calls.Add(1)
				select {
				case <-release:
					return []provider.Item{item}, nil
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			}}
		}
		results := make(chan error, 4)
		for range 4 {
			go func() { _, err := s.Refresh(context.Background()); results <- err }()
		}
		synctest.Wait()
		if calls.Load() != 1 {
			t.Fatalf("concurrent refresh calls = %d", calls.Load())
		}
		close(release)
		synctest.Wait()
		for range 4 {
			if err := <-results; err != nil {
				t.Fatal(err)
			}
		}
		// A second operation must be interrupted by the application's lifetime.
		a.client = func(config.Instance, ...config.Repo) provider.Client {
			return fakeClient{list: func(ctx context.Context) ([]provider.Item, error) { <-ctx.Done(); return nil, ctx.Err() }}
		}
		go func() { _, err := s.Refresh(context.Background()); results <- err }()
		synctest.Wait()
		s.Close()
		synctest.Wait()
		if err := <-results; !errors.Is(err, ErrClosed) {
			t.Fatalf("closed refresh = %v", err)
		}
	})
}
