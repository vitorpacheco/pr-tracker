package ui

import (
	"errors"
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
	if !tabs[6].match(issue) || tabs[4].match(issue) || tabs[3].match(issue) {
		t.Fatal("issue tab matching")
	}
	if !tabs[2].match(pr) || !tabs[3].match(pr) || tabs[4].match(pr) {
		t.Fatal("PR tab matching")
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
