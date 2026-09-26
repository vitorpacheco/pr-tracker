package provider

import (
	"path/filepath"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

func TestAccumulatorMergesRelations(t *testing.T) {
	a := newAccumulator()
	pr := Item{Instance: "gh", Repo: "o/r", Number: 1}
	a.add(pr, ReviewRequested)
	a.add(pr, Assigned)
	a.add(Item{Instance: "gh", Repo: "o/r", Number: 2}, Authored)
	a.add(Item{Instance: "gh", Repo: "o/r", Number: 3}, 0)
	got := a.list()
	if len(got) != 3 || got[0].Relations != ReviewRequested|Assigned || got[1].Relations != Authored || got[2].Relations != 0 {
		t.Fatalf("got %+v", got)
	}
}

func TestNewPassesTrackedReposForInstance(t *testing.T) {
	in := config.Instance{Name: "work", Provider: config.GitLab, Host: "gitlab.example.com"}
	c := New(in,
		config.Repo{Instance: "work", Name: "group/all", TrackAll: true},
		config.Repo{Instance: "work", Name: "group/related-only"},
		config.Repo{Instance: "other", Name: "group/other", TrackAll: true},
	)
	g, ok := c.(*gitlab)
	if !ok || len(g.tracked) != 1 || g.tracked[0] != "group/all" {
		t.Fatalf("client = %#v", c)
	}
}

func TestGitHubConvert(t *testing.T) {
	g := &github{in: config.Instance{Name: "ghe", Provider: config.GitHub, Host: "ghe.corp"}}
	var n ghPR
	n.Number = 7
	n.Repository.NameWithOwner = "o/r"
	n.ReviewDecision = "CHANGES_REQUESTED"
	n.Mergeable = "CONFLICTING"
	n.LatestReviews.Nodes = append(n.LatestReviews.Nodes, struct {
		State  string `json:"state"`
		Author *struct {
			Login string `json:"login"`
		} `json:"author"`
	}{State: "APPROVED", Author: &struct {
		Login string `json:"login"`
	}{Login: "Me"}})
	pr := g.convert(n, "me")
	if pr.Review != ReviewChangesRequested || !pr.Conflicts || !pr.ApprovedByMe || pr.Host != "ghe.corp" || pr.Ref() != "#7" {
		t.Fatalf("got %+v", pr)
	}
	if g.repoArg(&pr) != "ghe.corp/o/r" || g.HeadRef(&pr) != "refs/pull/7/head" {
		t.Fatal("unexpected repo arg / head ref")
	}
}

func TestStates(t *testing.T) {
	for in, want := range map[string]CIState{"SUCCESS": CISuccess, "FAILURE": CIFailure, "PENDING": CIPending, "SKIPPED": CICanceled, "": CINone} {
		if got := ghState(in); got != want {
			t.Errorf("ghState(%q) = %q, want %q", in, got, want)
		}
	}
	for in, want := range map[string]CIState{"SUCCESS": CISuccess, "failed": CIFailure, "RUNNING": CIPending, "MANUAL": CIPending, "CANCELED": CICanceled} {
		if got := glState(in); got != want {
			t.Errorf("glState(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestGitLabConvert(t *testing.T) {
	g := &gitlab{in: config.Instance{Name: "corp", Provider: config.GitLab, Host: "gitlab.corp"}}
	var n glMR
	n.IID = "42"
	n.Project.FullPath = "grp/proj"
	n.Project.WebURL = "https://gitlab.corp/grp/proj"
	n.SourceProject = &struct {
		FullPath string `json:"fullPath"`
	}{FullPath: "me/proj"}
	pr := g.convert(n, "me")
	if pr.Number != 42 || !pr.FromFork || pr.Ref() != "!42" || pr.Review != ReviewRequired {
		t.Fatalf("got %+v", pr)
	}
	if g.repoArg(&pr) != "https://gitlab.corp/grp/proj" || g.HeadRef(&pr) != "refs/merge-requests/42/head" {
		t.Fatal("unexpected repo arg / head ref")
	}
}

func TestMissingTool(t *testing.T) {
	// An empty PATH still allows Homebrew discovery on macOS.
	ctx := toolchain.WithPaths(t.Context(), map[string]string{
		"gh": filepath.Join(t.TempDir(), "missing-gh"),
	})
	_, err := run(ctx, "", nil, "gh", "api")
	if !isMissing(err) {
		t.Fatalf("expected MissingToolError, got %v", err)
	}
}

func TestGitLabConvertIssue(t *testing.T) {
	g := &gitlab{in: config.Instance{Name: "corp", Provider: config.GitLab, Host: "gitlab.corp"}}
	it := g.convertIssue(glIssue{
		IID: "9", Reference: "grp/sub/proj#9", WebURL: "https://gitlab.corp/grp/sub/proj/-/issues/9",
		Notes: 3, Labels: glLabels{Nodes: []struct {
			Title string `json:"title"`
		}{{Title: "bug"}}},
	})
	if it.Repo != "grp/sub/proj" || it.RepoURL != "https://gitlab.corp/grp/sub/proj" || it.Number != 9 ||
		!it.IsIssue() || it.Ref() != "#9" || it.Comments != 3 || len(it.Labels) != 1 {
		t.Fatalf("got %+v", it)
	}
	mr := Item{Kind: KindPR, Instance: "corp", Repo: "grp/sub/proj", Number: 9}
	if it.Key() == mr.Key() {
		t.Fatal("issue and MR with the same number must have different keys")
	}
}
