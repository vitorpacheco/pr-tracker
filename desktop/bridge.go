package main

import (
	"context"
	"errors"
	"net/url"
	"os"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

type Item struct {
	Data     provider.Item
	Key      string
	Ref      string
	Clone    string
	Worktree string
}
type View struct {
	RefreshSeconds int
	Instances      []config.Instance
	Items          []Item
	Errors         map[string]string
	CacheError     string
	SyncedAt       time.Time
	Revision       uint64
}
type Request struct {
	Key         string
	Action      app.Action
	Body        string
	RemoveAfter bool
	Force       bool
}
type Result struct {
	Message      string
	Error        string
	Dirty        bool
	NeedsClone   bool
	ReloadThread bool
	View         View
}

// Bridge adapts shared application state to JSON-safe desktop models.
type Bridge struct {
	ready         <-chan struct{}
	terminal      func(string, []string) error
	folder        func(string) error
	pickFolder    func() (string, error)
	session       *app.Session
	openURL       func(string)
	startupError  string
	settingsError error
}

func (b *Bridge) Load() (View, error) {
	b.wait()
	state, err := b.session.Load(context.Background())
	return b.view(state), err
}
func (b *Bridge) Refresh() (View, error) {
	b.wait()
	state, err := b.session.Refresh(context.Background())
	return b.view(state), err
}
func (b *Bridge) Detail(key string) (*provider.Thread, error) {
	b.wait()
	return b.session.Detail(context.Background(), key)
}
func (b *Bridge) Execute(request Request) Result {
	b.wait()
	out, err := b.session.Execute(context.Background(), request.Key, app.Command{Action: request.Action, Body: request.Body, RemoveAfter: request.RemoveAfter, Force: request.Force})
	result := Result{Message: out.Message, ReloadThread: out.ReloadThread}
	if err == nil && (request.Action == app.PrepareTerminal || request.Action == app.PrepareDiff) {
		if b.terminal == nil {
			err = errors.New("terminal desktop indisponível")
		} else {
			err = b.terminal(out.Directory, out.Args)
			if err == nil {
				result.Message = "Terminal aberto"
			}
		}
	}
	if err != nil {
		result.Error = err.Error()
		result.Dirty = errors.Is(err, app.ErrDirtyWorktree)
		result.NeedsClone = errors.Is(err, app.ErrCloneRequired)
	}
	state, loadErr := b.session.Load(context.Background())
	if out.Refresh {
		state, loadErr = b.session.Refresh(context.Background())
	}
	if loadErr != nil && result.Error == "" {
		result.Error = loadErr.Error()
	}
	result.View = b.view(state)
	return result
}
func (b *Bridge) Settings() config.Config { b.wait(); return b.session.Configuration() }
func (b *Bridge) SaveSettings(cfg config.Config) (View, error) {
	b.wait()
	if b.settingsError != nil {
		return View{}, b.settingsError
	}
	if err := b.session.SaveSettings(cfg); err != nil {
		return View{}, err
	}
	return b.Refresh()
}
func (b *Bridge) Diagnose() (app.Diagnostics, error) {
	b.wait()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	return b.session.Diagnose(ctx)
}
func (b *Bridge) OpenItem(key string) error {
	b.wait()
	state, err := b.session.Load(context.Background())
	if err != nil {
		return err
	}
	for _, item := range state.Items {
		if item.Key() != key {
			continue
		}
		parsed, err := url.Parse(item.URL)
		if err != nil || (parsed.Scheme != "https" && parsed.Scheme != "http") || parsed.Host == "" {
			return errors.New("link inválido")
		}
		if b.openURL == nil {
			return errors.New("navegador indisponível")
		}
		b.openURL(item.URL)
		return nil
	}
	return errors.New("item não encontrado")
}
func (b *Bridge) view(state app.State) View {
	cfg := b.session.Configuration()
	result := View{RefreshSeconds: int(cfg.Interval().Seconds()), Instances: cfg.Instances, Items: []Item{}, Errors: map[string]string{}, SyncedAt: state.SyncedAt, Revision: state.Revision, CacheError: b.startupError}
	if state.CacheError != nil {
		result.CacheError = state.CacheError.Error()
	}
	for name, err := range state.Errors {
		result.Errors[name] = err.Error()
	}
	for _, item := range state.Items {
		result.Items = append(result.Items, Item{Data: item, Key: item.Key(), Ref: item.Ref(), Clone: state.Clones[item.Key()], Worktree: state.Worktrees[item.Key()]})
	}
	return result
}

func (b *Bridge) MergeOptions(key string) (provider.MergeOptions, error) {
	b.wait()
	return b.session.MergeOptions(key)
}
func (b *Bridge) PickFolder() (string, error) {
	b.wait()
	if b.pickFolder == nil {
		return "", errors.New("seletor de pasta indisponível")
	}
	return b.pickFolder()
}
func (b *Bridge) OpenFolder(key string) error {
	b.wait()
	state, err := b.session.Load(context.Background())
	if err != nil {
		return err
	}
	dir := state.Worktrees[key]
	if dir == "" {
		dir = state.Clones[key]
	}
	if dir == "" {
		return app.ErrCloneRequired
	}
	info, err := os.Stat(dir)
	if err != nil {
		return err
	}
	if !info.IsDir() {
		return errors.New("pasta não encontrada")
	}
	if b.folder == nil {
		return errors.New("gerenciador de arquivos indisponível")
	}
	return b.folder(dir)
}

func (b *Bridge) wait() {
	if b.ready != nil {
		<-b.ready
	}
}
