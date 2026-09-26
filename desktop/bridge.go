package main

import (
	"context"
	"errors"
	"net/url"
	"os"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/theme"
)

type Item struct {
	Data     provider.Item
	Key      string
	Ref      string
	Clone    string
	Worktree string
}
type View struct {
	Language       string
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
	pickTheme     func() (string, error)
	saveTheme     func() (string, error)
	session       *app.Session
	openURL       func(string)
	startupError  string
	settingsError error
}

func (b *Bridge) Theme() (theme.Palette, error) {
	b.wait()
	cfg := b.session.Configuration()
	p, err := theme.Resolve(cfg.ThemePath())
	return p, b.localizedError(err)
}
func (b *Bridge) PickTheme() (string, error) {
	b.wait()
	if b.pickTheme == nil {
		return "", errors.New(b.t("Seletor de arquivos indisponível"))
	}
	return b.pickTheme()
}
func (b *Bridge) ExportTheme(p theme.Palette) (string, error) {
	b.wait()
	if b.saveTheme == nil {
		return "", errors.New(b.t("Seletor de arquivos indisponível"))
	}
	path, err := b.saveTheme()
	if err != nil || path == "" {
		return "", b.localizedError(err)
	}
	if err := theme.Export(path, p); err != nil {
		return "", b.localizedError(err)
	}
	return path, nil
}

func (b *Bridge) Load() (View, error) {
	b.wait()
	state, err := b.session.Load(context.Background())
	return b.view(state), b.localizedError(err)
}
func (b *Bridge) Refresh() (View, error) {
	b.wait()
	state, err := b.session.Refresh(context.Background())
	return b.view(state), b.localizedError(err)
}
func (b *Bridge) Detail(key string) (*provider.Thread, error) {
	b.wait()
	thread, err := b.session.Detail(context.Background(), key)
	return thread, b.localizedError(err)
}
func (b *Bridge) Execute(request Request) Result {
	b.wait()
	out, err := b.session.Execute(context.Background(), request.Key, app.Command{Action: request.Action, Body: request.Body, RemoveAfter: request.RemoveAfter, Force: request.Force})
	result := Result{Message: out.Message, ReloadThread: out.ReloadThread}
	if err == nil && (request.Action == app.PrepareTerminal || request.Action == app.PrepareDiff) {
		if b.terminal == nil {
			err = errors.New(b.t("terminal desktop indisponível"))
		} else {
			err = b.terminal(out.Directory, out.Args)
			if err == nil {
				result.Message = b.t("Terminal aberto")
			}
		}
	}
	if err != nil {
		result.Error = b.errorText(err)
		result.Dirty = errors.Is(err, app.ErrDirtyWorktree)
		result.NeedsClone = errors.Is(err, app.ErrCloneRequired)
	}
	state, loadErr := b.session.Load(context.Background())
	if out.Refresh {
		state, loadErr = b.session.Refresh(context.Background())
	}
	if loadErr != nil && result.Error == "" {
		result.Error = b.errorText(loadErr)
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
		return View{}, b.localizedError(err)
	}
	return b.Refresh()
}
func (b *Bridge) Diagnose() (app.Diagnostics, error) {
	b.wait()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	diagnostics, err := b.session.Diagnose(ctx)
	if err != nil {
		return diagnostics, b.localizedError(err)
	}
	return diagnostics, nil
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
			return errors.New(b.t("link inválido"))
		}
		if b.openURL == nil {
			return errors.New(b.t("navegador indisponível"))
		}
		b.openURL(item.URL)
		return nil
	}
	return errors.New(b.t("item não encontrado"))
}
func (b *Bridge) view(state app.State) View {
	cfg := b.session.Configuration()
	result := View{Language: i18n.Resolve(cfg.Language), RefreshSeconds: int(cfg.Interval().Seconds()), Instances: cfg.Instances, Items: []Item{}, Errors: map[string]string{}, SyncedAt: state.SyncedAt, Revision: state.Revision, CacheError: b.startupError}
	if state.CacheError != nil {
		result.CacheError = b.errorText(state.CacheError)
	}
	for name, err := range state.Errors {
		result.Errors[name] = b.errorText(err)
	}
	for _, item := range state.Items {
		result.Items = append(result.Items, Item{Data: item, Key: item.Key(), Ref: item.Ref(), Clone: state.Clones[item.Key()], Worktree: state.Worktrees[item.Key()]})
	}
	return result
}

func (b *Bridge) MergeOptions(key string) (provider.MergeOptions, error) {
	b.wait()
	options, err := b.session.MergeOptions(key)
	return options, b.localizedError(err)
}
func (b *Bridge) PickFolder() (string, error) {
	b.wait()
	if b.pickFolder == nil {
		return "", errors.New(b.t("seletor de pasta indisponível"))
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
		return b.localizedError(app.ErrCloneRequired)
	}
	info, err := os.Stat(dir)
	if err != nil {
		return err
	}
	if !info.IsDir() {
		return errors.New(b.t("pasta não encontrada"))
	}
	if b.folder == nil {
		return errors.New(b.t("gerenciador de arquivos indisponível"))
	}
	return b.localizedError(b.folder(dir))
}

func (b *Bridge) wait() {
	if b.ready != nil {
		<-b.ready
	}
}

func (b *Bridge) t(message string) string {
	cfg := b.session.Configuration()
	return i18n.Text(i18n.Resolve(cfg.Language), message)
}

func (b *Bridge) errorText(err error) string {
	cfg := b.session.Configuration()
	return i18n.ErrorText(i18n.Resolve(cfg.Language), err)
}

func (b *Bridge) localizedError(err error) error {
	if err == nil {
		return nil
	}
	return errors.New(b.errorText(err))
}
