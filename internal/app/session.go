package app

import (
	"context"
	"errors"
	"maps"
	"reflect"
	"slices"
	"sync"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

var ErrClosed = i18n.Errorf("aplicação encerrada")
var ErrBusy = i18n.Errorf("já existe uma ação em andamento para este item")

// State is a detached snapshot. Adapters own selection and rendering only.
type State struct {
	Snapshot
	Worktrees map[string]string
	SyncedAt  time.Time
	Revision  uint64
}

type refreshCall struct {
	done     chan struct{}
	revision uint64
}

// Session owns application state and lifetime. Concurrent refresh callers share
// one operation; cancelling a caller does not cancel the other waiters.
type Session struct {
	cleanup     map[string]provider.Item
	mu          sync.Mutex
	application *Application
	state       State
	loaded      bool
	revision    uint64
	flight      *refreshCall
	busy        map[string]bool
	ctx         context.Context
	cancel      context.CancelFunc
	workers     sync.WaitGroup
	closed      bool
}

func NewSession(cfg *config.Config, store *cache.Store) *Session {
	ctx, cancel := context.WithCancel(context.Background())
	return &Session{application: New(cfg, store), cleanup: map[string]provider.Item{}, busy: map[string]bool{}, ctx: ctx, cancel: cancel}
}

func (s *Session) Close() {
	s.cancel()
	s.mu.Lock()
	s.closed = true
	s.mu.Unlock()
	s.workers.Wait()
}

func (s *Session) Configuration() config.Config {
	s.mu.Lock()
	defer s.mu.Unlock()
	return cloneConfig(s.application.cfg)
}

func cloneConfig(cfg config.Config) config.Config {
	cfg.ToolPaths = maps.Clone(cfg.ToolPaths)
	cfg.Instances = slices.Clone(cfg.Instances)
	cfg.Repos = slices.Clone(cfg.Repos)
	cfg.CloneRoots = slices.Clone(cfg.CloneRoots)
	return cfg
}

// Configure adopts already-persisted TUI settings. Desktop edits use SaveSettings.
func (s *Session) Configure(cfg *config.Config) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed || reflect.DeepEqual(s.application.cfg, *cfg) {
		return
	}
	s.configureLocked(cfg)
}

func (s *Session) configureLocked(cfg *config.Config) {
	old := s.application
	next := New(cfg, nil)
	next.store = old.store
	s.application = next
	s.revision++
	s.loaded = false
	// Never retain cached items under a changed host/provider identity.
	items := s.state.Items[:0]
	for _, item := range s.state.Items {
		in, ok := cfg.Instance(item.Instance)
		if ok && !in.Disabled && in.Host == item.Host && in.Provider == item.Provider {
			items = append(items, item)
		}
	}
	s.state.Snapshot = next.Reconcile(Snapshot{Items: items}, RefreshResult{})
	s.state.Revision = s.revision
	s.scanWorktreesLocked()
}

// SaveSettings commits a validated copy before changing live state. Failure
// leaves both the running configuration and its generation untouched.
func (s *Session) SaveSettings(cfg config.Config) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return ErrClosed
	}
	next := cloneConfig(s.application.cfg)
	next.Language = cfg.Language
	tr := func(message string) string { return i18n.Text(i18n.Resolve(cfg.Language), message) }
	next.ToolPaths = maps.Clone(cfg.ToolPaths)
	next.DesktopTerminal = cfg.DesktopTerminal
	next.RefreshInterval, next.Terminal, next.DiffTool = cfg.RefreshInterval, cfg.Terminal, cfg.DiffTool
	next.WorktreeDir = cfg.WorktreeDir
	next.CloneRoots = slices.Clone(cfg.CloneRoots)
	next.Instances = slices.Clone(cfg.Instances)
	next.Repos = slices.Clone(cfg.Repos)
	for i := range next.Instances {
		in := &next.Instances[i]
		if in.Host == "" {
			in.Host = in.Provider.DefaultHost()
		}
		in.Host = config.NormalizeHost(in.Host)
		if in.Name == "" {
			in.Name = in.Host
		}
	}
	if err := next.Validate(); err != nil {
		return err
	}
	for _, repo := range next.Repos {
		if _, ok := next.Instance(repo.Instance); !ok {
			return errors.New(tr("repositório referencia uma instância inexistente: ") + repo.Instance)
		}
		if repo.Name == "" {
			return errors.New(tr("nome do repositório é obrigatório"))
		}
		switch repo.MergeMethod {
		case "", "merge", "squash", "rebase":
		default:
			return errors.New(tr("método de merge inválido no repositório"))
		}
		previous, ok := s.application.cfg.Repo(repo.Instance, repo.Name)
		if repo.Path != "" && (!ok || previous.Path != repo.Path) {
			ctx, cancel := context.WithTimeout(s.ctx, 15*time.Second)
			err := gitops.ValidateClone(toolchain.WithPaths(ctx, next.ToolPaths), repo.Path)
			cancel()
			if err != nil {
				return err
			}
		}
	}
	if err := next.Save(); err != nil {
		return err
	}
	s.configureLocked(&next)
	return nil
}

func (s *Session) Load(ctx context.Context) (State, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return State{}, ErrClosed
	}
	if err := ctx.Err(); err != nil {
		return State{}, err
	}
	if !s.loaded {
		work, cancel := context.WithTimeout(s.ctx, 10*time.Second)
		stop := context.AfterFunc(ctx, cancel)
		cached, err := s.application.Load(work)
		stop()
		cancel()
		if ctx.Err() != nil {
			return State{}, ctx.Err()
		}
		for _, item := range s.state.Items {
			delete(cached.Items, item.Instance)
		}
		s.state.Snapshot = s.application.Reconcile(s.state.Snapshot, RefreshResult{Items: cached.Items})
		s.state.CacheError = err
		if cached.SyncedAt.After(s.state.SyncedAt) {
			s.state.SyncedAt = cached.SyncedAt
		}
		s.state.Revision = s.revision
		s.scanWorktreesLocked()
		s.loaded = true
	}
	return copyState(s.state), nil
}

func (s *Session) Refresh(ctx context.Context) (State, error) {
	if _, err := s.Load(ctx); err != nil {
		return State{}, err
	}
	for {
		s.mu.Lock()
		if s.closed {
			s.mu.Unlock()
			return State{}, ErrClosed
		}
		if err := ctx.Err(); err != nil {
			s.mu.Unlock()
			return State{}, err
		}
		call := s.flight
		if call == nil {
			call = &refreshCall{done: make(chan struct{}), revision: s.revision}
			s.flight = call
			a := s.application
			s.workers.Add(1)
			go s.refresh(a, call)
		}
		s.mu.Unlock()
		select {
		case <-ctx.Done():
			return State{}, ctx.Err()
		case <-s.ctx.Done():
			return State{}, ErrClosed
		case <-call.done:
		}
		s.mu.Lock()
		if s.closed {
			s.mu.Unlock()
			return State{}, ErrClosed
		}
		current := call.revision == s.revision
		state := copyState(s.state)
		s.mu.Unlock()
		if current {
			return state, nil
		}
		// Settings changed while the old generation was querying providers.
	}
}

func (s *Session) refresh(a *Application, call *refreshCall) {
	defer s.workers.Done()
	ctx, cancel := context.WithTimeout(s.ctx, 2*time.Minute)
	defer cancel()
	result := a.Refresh(ctx)
	s.mu.Lock()
	defer s.mu.Unlock()
	if !s.closed && call.revision == s.revision {
		s.state.Snapshot = a.Reconcile(s.state.Snapshot, result)
		if len(result.Items) > 0 {
			s.state.SyncedAt = time.Now()
		}
		s.state.Revision = s.revision
		s.loaded = true
		s.scanWorktreesLocked()
	}
	s.flight = nil
	close(call.done)
}

func (s *Session) scanWorktreesLocked() {
	s.state.Worktrees = map[string]string{}
	for i := range s.state.Items {
		if dir, ok := s.application.worktreeExists(&s.application.cfg, &s.state.Items[i]); ok {
			s.state.Worktrees[s.state.Items[i].Key()] = dir
		}
	}
}

func (s *Session) itemLocked(key string) (provider.Item, error) {
	for _, item := range s.state.Items {
		if item.Key() == key {
			return item, nil
		}
	}
	return provider.Item{}, errors.New(s.application.t("item não encontrado; atualize a lista"))
}

func (s *Session) Detail(ctx context.Context, key string) (*provider.Thread, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil, ErrClosed
	}
	item, err := s.itemLocked(key)
	a := s.application
	if err != nil {
		s.mu.Unlock()
		return nil, err
	}
	s.workers.Add(1)
	s.mu.Unlock()
	defer s.workers.Done()
	work, cancel := context.WithTimeout(s.ctx, time.Minute)
	defer cancel()
	stop := context.AfterFunc(ctx, cancel)
	defer stop()
	return a.Detail(work, item)
}

// Execute resolves the item from trusted state. Adapters cannot substitute a
// different repository or URL in a command sent from the frontend.
func (s *Session) Execute(ctx context.Context, key string, command Command) (Outcome, error) {
	if err := ctx.Err(); err != nil {
		return Outcome{}, err
	}
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return Outcome{}, ErrClosed
	}
	item, err := s.itemLocked(key)
	if err != nil && command.Action == RemoveWorktree {
		if pending, ok := s.cleanup[key]; ok {
			item, err = pending, nil
		}
	}
	if err != nil {
		s.mu.Unlock()
		return Outcome{}, err
	}
	if s.busy[key] {
		s.mu.Unlock()
		return Outcome{}, ErrBusy
	}
	s.busy[key] = true
	a := s.application
	s.workers.Add(1)
	s.mu.Unlock()
	defer s.workers.Done()
	defer func() { s.mu.Lock(); delete(s.busy, key); s.scanWorktreesLocked(); s.mu.Unlock() }()
	work, cancel := context.WithTimeout(s.ctx, 5*time.Minute)
	defer cancel()
	stop := context.AfterFunc(ctx, cancel)
	defer stop()
	command.Item = item
	out, err := a.Execute(work, command)
	s.mu.Lock()
	if errors.Is(err, ErrDirtyWorktree) {
		s.cleanup[key] = item
	}
	if command.Action == RemoveWorktree && err == nil {
		delete(s.cleanup, key)
	}
	s.mu.Unlock()
	return out, err
}

func copyState(state State) State {
	state.Items = slices.Clone(state.Items)
	for i := range state.Items {
		item := &state.Items[i]
		item.Labels = slices.Clone(item.Labels)
		item.Assignees = slices.Clone(item.Assignees)
		item.ApprovedBy = slices.Clone(item.ApprovedBy)
		item.Checks = slices.Clone(item.Checks)
	}
	state.Errors = maps.Clone(state.Errors)
	state.Clones = maps.Clone(state.Clones)
	state.Worktrees = maps.Clone(state.Worktrees)
	return state
}

func (s *Session) MergeOptions(key string) (provider.MergeOptions, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return provider.MergeOptions{}, ErrClosed
	}
	item, err := s.itemLocked(key)
	if err != nil {
		return provider.MergeOptions{}, err
	}
	return s.application.MergeOptions(item), nil
}

func (s *Session) Worktrees() map[string]string {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.scanWorktreesLocked()
	return maps.Clone(s.state.Worktrees)
}
