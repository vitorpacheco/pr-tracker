package provider

import (
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

func TestAccumulatorMergesRelations(t *testing.T) {
	a := newAccumulator()
	pr := PR{Instance: "gh", Repo: "o/r", Number: 1}
	a.add(pr, ReviewRequested)
	a.add(pr, Assigned)
	a.add(PR{Instance: "gh", Repo: "o/r", Number: 2}, Authored)
	got := a.list()
	if len(got) != 2 || got[0].Relations != ReviewRequested|Assigned || got[1].Relations != Authored {
		t.Fatalf("got %+v", got)
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
	t.Setenv("PATH", t.TempDir())
	_, err := run(t.Context(), "", nil, "gh", "api")
	if !isMissing(err) {
		t.Fatalf("expected MissingToolError, got %v", err)
	}
}
