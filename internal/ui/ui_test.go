package ui

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

func TestCleanMarkdown(t *testing.T) {
	in := "<!-- bot meta -->\r\nHello <sub>world</sub>\n\n\n\n<details><summary>More</summary>\n\nhidden `a <T> b`\n</details>"
	got := cleanMarkdown(in)
	for _, bad := range []string{"<!--", "<sub>", "<details>", "\n\n\n"} {
		if strings.Contains(got, bad) {
			t.Fatalf("%q still contains %q", got, bad)
		}
	}
	if !strings.Contains(got, "**▸ More**") || !strings.Contains(got, "a <T> b") {
		t.Fatalf("unexpected result %q", got)
	}
	if cleanMarkdown("<!-- only a marker -->") != "" {
		t.Fatal("marker-only body should be empty")
	}
}

func TestTabMatch(t *testing.T) {
	issue := &provider.Item{Kind: provider.KindIssue, Relations: provider.Mentioned}
	pr := &provider.Item{Kind: provider.KindPR, Relations: provider.Assigned}
	unrelated := &provider.Item{Kind: provider.KindPR}
	if !tabs[6].match(issue) || tabs[4].match(issue) || tabs[3].match(issue) {
		t.Fatal("issue tab matching")
	}
	if !tabs[2].match(pr) || !tabs[3].match(pr) || tabs[4].match(pr) {
		t.Fatal("PR tab matching")
	}
	if tabs[0].match(unrelated) || tabs[1].match(unrelated) || tabs[2].match(unrelated) || !tabs[3].match(unrelated) {
		t.Fatal("unrelated tracked PR must appear only in the all tab")
	}
}
func TestCloseWithoutMerge(t *testing.T) {
	cfg := config.Default()
	cfg.Language = "pt"
	cfg.Instances = []config.Instance{{Name: "gh", Provider: config.GitHub, Host: "github.com"}}
	m := New(cfg, nil)
	pr := provider.Item{Kind: provider.KindPR, Instance: "gh", Provider: config.GitHub, Repo: "o/r", Number: 3, SourceBranch: "feat"}

	m.openMenu(&pr)
	var keys []string
	for _, it := range m.menu {
		keys = append(keys, it.key)
	}
	if !strings.Contains(strings.Join(keys, ""), "X") || !strings.Contains(strings.Join(keys, ""), "C") {
		t.Fatalf("menu keys %v lack close actions", keys)
	}
	m.modal = modalNone

	if m.prAction(&pr, "C"); m.modal != modalNone {
		t.Fatal("close and remove worktree needs an existing worktree")
	}
	m.prAction(&pr, "X")
	if m.modal != modalConfirm || !strings.Contains(m.confirm.title, "Fechar #3") {
		t.Fatalf("modal = %v, confirm = %+v", m.modal, m.confirm)
	}

	issue := provider.Item{Kind: provider.KindIssue, Instance: "gh", Repo: "o/r", Number: 4}
	m.modal, m.confirm = modalNone, nil
	if m.prAction(&issue, "X"); m.modal != modalNone {
		t.Fatal("issues cannot be closed as PRs")
	}
}

func TestTrackAllRepoActionPersistsAndToggles(t *testing.T) {
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	cfg := config.Default()
	cfg.Language = "pt"
	cfg.Instances = []config.Instance{{Name: "gl", Provider: config.GitLab, Host: "gitlab.example.com"}}
	m := New(cfg, nil)
	pr := provider.Item{Kind: provider.KindPR, Instance: "gl", Provider: config.GitLab, Repo: "group/project", Number: 7}

	m.openMenu(&pr)
	found := false
	for _, item := range m.menu {
		if item.key == "R" && strings.Contains(item.label, "todos os MRs") {
			found = true
		}
	}
	if !found {
		t.Fatalf("track-all action missing from menu: %+v", m.menu)
	}
	m.modal = modalNone
	m.prAction(&pr, "R")
	repo, ok := cfg.Repo("gl", "group/project")
	if !ok || !repo.TrackAll || repo.Path != "" {
		t.Fatalf("repo = %+v, found = %v", repo, ok)
	}
	loaded, _, err := config.Load()
	if err != nil {
		t.Fatal(err)
	}
	if repo, ok := loaded.Repo("gl", "group/project"); !ok || !repo.TrackAll {
		t.Fatalf("persisted repo = %+v, found = %v", repo, ok)
	}

	m.prAction(&pr, "R")
	if _, ok := cfg.Repo("gl", "group/project"); ok {
		t.Fatal("track-only repo remained after toggling off")
	}
	if !m.refreshQueued {
		t.Fatal("configuration change during refresh did not queue another refresh")
	}
}

func TestSnapshotSelectsNonEmptyTabAndPreservesSyncStatus(t *testing.T) {
	cfg := config.Default()
	cfg.Language = "pt"
	cfg.WorktreeDir = t.TempDir()
	m := New(cfg, nil)
	defer m.Close()
	syncedAt := time.Date(2026, 9, 19, 12, 0, 0, 0, time.UTC)
	item := provider.Item{Instance: "work", Kind: provider.KindPR, Relations: provider.Authored}
	m.Update(stateMsg{state: app.State{Snapshot: app.Snapshot{Items: []provider.Item{item}, Errors: map[string]error{"work": errors.New("offline")}}, SyncedAt: syncedAt}, refresh: true, gen: m.gen})
	if m.tab != 1 {
		t.Fatalf("tab = %d, want authored", m.tab)
	}
	if !m.lastSync.Equal(syncedAt) || m.instErr["work"] == nil || len(m.prs) != 1 {
		t.Fatal("snapshot lost cached items or offline status")
	}
}

func TestActionOutcomePreservesPartialSuccessAndDirtyConfirmation(t *testing.T) {
	cfg := config.Default()
	cfg.Language = "pt"
	cfg.WorktreeDir = t.TempDir()
	cfg.Instances = []config.Instance{{Name: "work", Provider: config.GitHub, Host: "github.com"}}
	item := provider.Item{Instance: "work", Provider: config.GitHub, Host: "github.com", Repo: "org/repo", Number: 7}
	m := New(cfg, nil)
	m.pending[item.Key()] = "aprovando"
	msg := actionResult(item, "A", app.Outcome{Message: "aprovado #7", Refresh: true}, app.ErrDirtyWorktree).(actionDoneMsg)
	if msg.err != nil || msg.dirty == nil || !msg.refresh || msg.ok != "aprovado #7" {
		t.Fatalf("message = %+v", msg)
	}
	_, cmd := m.Update(msg)
	if m.modal != modalConfirm || m.confirm == nil || m.confirm.yes != "Remover mesmo assim" {
		t.Fatal("missing separate confirmation for dirty worktree")
	}
	if m.status != "aprovado #7" || cmd == nil || !m.loading {
		t.Fatal("remote success did not trigger refresh")
	}
	if _, busy := m.pending[item.Key()]; busy {
		t.Fatal("action remained pending")
	}
}

func TestActionOutcomeRequestsCloneAndPreservesCleanupError(t *testing.T) {
	item := provider.Item{Instance: "work", Repo: "org/repo", Number: 7}
	missing := actionResult(item, "d", app.Outcome{}, app.ErrCloneRequired).(needPathMsg)
	if missing.pr.Key() != item.Key() || missing.action != "d" {
		t.Fatalf("missing clone message = %+v", missing)
	}
	failure := errors.New("cleanup failed")
	msg := actionResult(item, "M", app.Outcome{Message: "merge feito", Refresh: true}, failure).(actionDoneMsg)
	if !errors.Is(msg.err, failure) || !msg.refresh || msg.ok != "merge feito" || msg.dirty != nil {
		t.Fatalf("partial success = %+v", msg)
	}
}

func TestLanguageSettingsApplyImmediately(t *testing.T) {
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	t.Setenv("LC_ALL", "pt_BR.UTF-8")
	t.Setenv("LANGUAGE", "")
	cfg := config.Default()
	m := New(cfg, nil)
	defer m.Close()
	if m.language != "pt" {
		t.Fatalf("system language = %s", m.language)
	}
	m.openSettingsForm()
	language := m.form.get("Idioma")
	language.choice = 1 // English
	if err := m.form.submit(m.form); err != nil {
		t.Fatal(err)
	}
	if m.language != "en" || cfg.Language != "en" || m.status != "settings saved" {
		t.Fatalf("language change not applied: %+v", m)
	}
	if !strings.Contains(m.filter.Placeholder, "filter by title") {
		t.Fatal("placeholder not updated")
	}
	m.openSettingsForm()
	if got := m.form.render(&zones{}, m.t, m.styles); !strings.Contains(got, "Language") || !strings.Contains(got, "System") || !strings.Contains(got, "Save ctrl+s") {
		t.Fatalf("settings not translated: %s", got)
	}
	loaded, _, err := config.Load()
	if err != nil || loaded.Language != "en" {
		t.Fatalf("saved config = %+v, %v", loaded, err)
	}
	m.form.get("Idioma").choice = 0 // Follow system again.
	if err := m.form.submit(m.form); err != nil {
		t.Fatal(err)
	}
	if m.language != "pt" || cfg.Language != "system" {
		t.Fatal("system language not restored")
	}
}

func TestLocalizedRenderingPreservesRepositoryContent(t *testing.T) {
	for _, language := range []string{"en", "pt"} {
		t.Run(language, func(t *testing.T) {
			cfg := config.Default()
			cfg.Language = language
			m := New(cfg, nil)
			defer m.Close()
			item := provider.Item{Title: "Configurações", Repo: "org/repo", Author: "Nome", Kind: provider.KindPR}
			detail := strings.Join(m.detail(&item, 100, 30), "\n")
			if !strings.Contains(detail, "Configurações") || !strings.Contains(detail, "Nome") {
				t.Fatalf("repository content was translated: %s", detail)
			}
			m.openMenu(&item)
			want := "View conversation"
			if language == "pt" {
				want = "Ver conversa"
			}
			if !strings.Contains(m.menu[0].label, want) {
				t.Fatalf("menu = %+v", m.menu)
			}
		})
	}
}
