package ui

import (
	"strings"
	"testing"

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
