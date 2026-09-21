package ui

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

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

func TestCachedItemsSurviveFailedRefresh(t *testing.T) {
	cfg := config.Default()
	cfg.WorktreeDir = t.TempDir()
	cfg.Instances = []config.Instance{{Name: "work", Provider: config.GitHub, Host: "github.example.com"}}
	m := New(cfg, nil)
	cachedAt := time.Date(2026, 9, 19, 12, 0, 0, 0, time.UTC)
	item := provider.Item{Instance: "work", Provider: config.GitHub, Host: "github.example.com", Repo: "acme/app", Number: 7}
	m.Update(cacheMsg{
		prs:      map[string][]provider.Item{"work": {item}},
		syncedAt: cachedAt,
	})

	m.Update(refreshMsg{
		gen:       m.gen,
		prs:       map[string][]provider.Item{},
		errs:      map[string]error{"work": errors.New("test error")},
		cacheErrs: map[string]error{},
		clones:    map[string]string{},
	})
	if len(m.prs) != 1 || m.prs[0].Key() != item.Key() {
		t.Fatalf("failed refresh discarded cached items: %+v", m.prs)
	}
	if !m.lastSync.Equal(cachedAt) {
		t.Fatalf("lastSync = %v, want cached time %v", m.lastSync, cachedAt)
	}
}

func TestRefreshSelectsNonEmptyTabAfterEmptyCache(t *testing.T) {
	cfg := config.Default()
	cfg.WorktreeDir = t.TempDir()
	cfg.Instances = []config.Instance{{Name: "work", Provider: config.GitHub, Host: "github.example.com"}}
	m := New(cfg, nil)
	m.Update(cacheMsg{
		prs:      map[string][]provider.Item{"work": nil},
		syncedAt: time.Date(2026, 9, 19, 12, 0, 0, 0, time.UTC),
	})
	m.Update(refreshMsg{
		gen: m.gen,
		prs: map[string][]provider.Item{"work": {{
			Instance: "work", Kind: provider.KindPR, Relations: provider.Authored,
		}}},
		errs:      map[string]error{},
		cacheErrs: map[string]error{},
		clones:    map[string]string{},
	})
	if m.tab != 1 {
		t.Fatalf("tab = %d, want authored tab", m.tab)
	}
}
