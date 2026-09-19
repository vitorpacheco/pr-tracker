package ui

import (
	"strings"
	"testing"

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

func TestCloseWithoutMerge(t *testing.T) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "gh", Provider: config.GitHub, Host: "github.com"}}
	m := New(cfg)
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
