package provider

import (
	"encoding/json"
	"errors"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

func giteaClient() *gitea {
	return &gitea{
		in:   config.Instance{Name: "corp", Provider: config.Gitea, Host: "git.corp"},
		base: "https://git.corp",
	}
}

// searchPayload is trimmed from a real /repos/issues/search answer.
const searchPayload = `{
  "number": 6,
  "title": "reescreve a aplicação",
  "html_url": "https://git.corp/acme/app/pulls/6",
  "created_at": "2026-09-19T10:00:00-03:00",
  "updated_at": "2026-09-20T09:07:13-03:00",
  "comments": 3,
  "user": {"login": "ana", "username": "ana"},
  "labels": [{"name": "bug"}, {"name": "go"}],
  "assignees": [{"login": "bruno", "username": "bruno"}],
  "repository": {"id": 3, "name": "app", "owner": "acme", "full_name": "acme/app"},
  "pull_request": {"merged": false, "draft": true, "html_url": "https://git.corp/acme/app/pulls/6"}
}`

func TestGiteaConvert(t *testing.T) {
	var n gtIssue
	if err := json.Unmarshal([]byte(searchPayload), &n); err != nil {
		t.Fatal(err)
	}
	g := giteaClient()
	pr := g.convert(n, KindPR)
	switch {
	case pr.Repo != "acme/app":
		t.Fatalf("repo = %q", pr.Repo)
	case pr.RepoURL != "https://git.corp/acme/app":
		t.Fatalf("repo URL = %q", pr.RepoURL)
	case pr.Number != 6 || pr.Ref() != "#6":
		t.Fatalf("number = %d, ref = %q", pr.Number, pr.Ref())
	case pr.Author != "ana" || len(pr.Labels) != 2 || len(pr.Assignees) != 1:
		t.Fatalf("got %+v", pr)
	case !pr.Draft:
		t.Fatal("draft not carried over from pull_request")
	case pr.Comments != 3 || pr.UpdatedAt.IsZero():
		t.Fatalf("got %+v", pr)
	// The search says nothing about reviews, so a pull request starts as
	// pending until enrichment says otherwise.
	case pr.Review != ReviewRequired:
		t.Fatalf("review = %q", pr.Review)
	}
	iss := g.convert(n, KindIssue)
	if !iss.IsIssue() || iss.Review != "" || iss.Key() == pr.Key() {
		t.Fatalf("issue and pull request must not share a key: %q / %q", iss.Key(), pr.Key())
	}
	if g.HeadRef(&pr) != "refs/pull/6/head" {
		t.Fatalf("head ref = %q", g.HeadRef(&pr))
	}
}

func TestGiteaApplyPull(t *testing.T) {
	tests := []struct {
		name          string
		payload       string
		wantConflicts bool
		wantFork      bool
	}{{
		name: "mesmo repo, sem conflito",
		payload: `{"draft": false, "mergeable": true, "additions": 9, "deletions": 5, "changed_files": 2,
			"head": {"ref": "feat", "sha": "abc", "repo": {"full_name": "acme/app"}},
			"base": {"ref": "main", "sha": "def", "repo": {"full_name": "acme/app"}}}`,
	}, {
		name: "conflito",
		payload: `{"draft": false, "mergeable": false,
			"head": {"ref": "feat", "repo": {"full_name": "acme/app"}},
			"base": {"ref": "main", "repo": {"full_name": "acme/app"}}}`,
		wantConflicts: true,
	}, {
		// Gitea reports every draft as not mergeable; flagging them all as
		// conflicting would be wrong.
		name: "draft não é conflito",
		payload: `{"draft": true, "mergeable": false,
			"head": {"ref": "feat", "repo": {"full_name": "acme/app"}},
			"base": {"ref": "main", "repo": {"full_name": "acme/app"}}}`,
	}, {
		name: "fork",
		payload: `{"draft": false, "mergeable": true,
			"head": {"ref": "feat", "repo": {"full_name": "ana/app"}},
			"base": {"ref": "main", "repo": {"full_name": "acme/app"}}}`,
		wantFork: true,
	}}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var d gtPull
			if err := json.Unmarshal([]byte(tt.payload), &d); err != nil {
				t.Fatal(err)
			}
			pr := Item{Kind: KindPR}
			applyPull(&pr, d)
			if pr.Conflicts != tt.wantConflicts || pr.FromFork != tt.wantFork {
				t.Fatalf("conflicts = %v, fork = %v; want %v / %v", pr.Conflicts, pr.FromFork, tt.wantConflicts, tt.wantFork)
			}
			if pr.SourceBranch != "feat" || pr.TargetBranch != "main" {
				t.Fatalf("branches = %q → %q", pr.SourceBranch, pr.TargetBranch)
			}
		})
	}
}

func TestGiteaApplyPullDiffStats(t *testing.T) {
	var d gtPull
	if err := json.Unmarshal([]byte(`{"mergeable": true, "additions": 9809, "deletions": 5715, "changed_files": 75}`), &d); err != nil {
		t.Fatal(err)
	}
	var pr Item
	applyPull(&pr, d)
	if pr.Additions != 9809 || pr.Deletions != 5715 || pr.Files != 75 {
		t.Fatalf("got %+v", pr)
	}
}

func TestGiteaApplyReviews(t *testing.T) {
	tests := []struct {
		name       string
		reviews    string
		wantReview string
		wantBy     []string
		wantByMe   bool
	}{{
		name:       "sem reviews",
		reviews:    `[]`,
		wantReview: ReviewRequired,
	}, {
		name:       "aprovado por mim, ignorando maiúsculas",
		reviews:    `[{"state": "APPROVED", "user": {"login": "Me"}}]`,
		wantReview: ReviewApproved,
		wantBy:     []string{"Me"},
		wantByMe:   true,
	}, {
		// The last review of each user decides, so an approval that came after
		// a rejection wins.
		name: "última review do usuário vence",
		reviews: `[{"state": "REQUEST_CHANGES", "user": {"login": "ana"}},
			{"state": "APPROVED", "user": {"login": "ana"}}]`,
		wantReview: ReviewApproved,
		wantBy:     []string{"ana"},
	}, {
		name: "alterações pedidas por um revisor bastam",
		reviews: `[{"state": "APPROVED", "user": {"login": "ana"}},
			{"state": "REQUEST_CHANGES", "user": {"login": "bruno"}}]`,
		wantReview: ReviewChangesRequested,
		wantBy:     []string{"ana"},
	}, {
		name:       "review descartada não conta",
		reviews:    `[{"state": "REQUEST_CHANGES", "dismissed": true, "user": {"login": "ana"}}]`,
		wantReview: ReviewRequired,
	}, {
		name:       "comentário não decide nada",
		reviews:    `[{"state": "COMMENT", "user": {"login": "ana"}}]`,
		wantReview: ReviewRequired,
	}}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var rs []gtReview
			if err := json.Unmarshal([]byte(tt.reviews), &rs); err != nil {
				t.Fatal(err)
			}
			pr := Item{Kind: KindPR}
			applyReviews(&pr, rs, "me")
			if pr.Review != tt.wantReview || pr.ApprovedByMe != tt.wantByMe {
				t.Fatalf("review = %q, byMe = %v; want %q / %v", pr.Review, pr.ApprovedByMe, tt.wantReview, tt.wantByMe)
			}
			if len(pr.ApprovedBy) != len(tt.wantBy) {
				t.Fatalf("approvedBy = %v, want %v", pr.ApprovedBy, tt.wantBy)
			}
			for i, want := range tt.wantBy {
				if pr.ApprovedBy[i] != want {
					t.Fatalf("approvedBy = %v, want %v", pr.ApprovedBy, tt.wantBy)
				}
			}
		})
	}
}

// statusPayload is trimmed from a real /commits/{sha}/status answer: Gitea
// Actions write a relative target_url.
const statusPayload = `{
  "state": "pending",
  "total_count": 2,
  "statuses": [
    {"status": "success", "context": "ci / lint (push)", "target_url": "/acme/app/actions/runs/1/jobs/1"},
    {"status": "pending", "context": "ci / test (push)", "target_url": "/acme/app/actions/runs/1/jobs/2"}
  ]
}`

func TestGiteaApplyStatus(t *testing.T) {
	var st gtStatus
	if err := json.Unmarshal([]byte(statusPayload), &st); err != nil {
		t.Fatal(err)
	}
	pr := Item{Kind: KindPR}
	giteaClient().applyStatus(&pr, st)
	if pr.CI != CIPending || len(pr.Checks) != 2 {
		t.Fatalf("got %q with %d checks", pr.CI, len(pr.Checks))
	}
	if pr.Checks[0].Name != "ci / lint (push)" || pr.Checks[0].State != CISuccess {
		t.Fatalf("got %+v", pr.Checks[0])
	}
	if pr.Checks[0].URL != "https://git.corp/acme/app/actions/runs/1/jobs/1" {
		t.Fatalf("relative target_url was not completed: %q", pr.Checks[0].URL)
	}

	var empty gtStatus
	if err := json.Unmarshal([]byte(`{"state": "pending", "total_count": 0, "statuses": []}`), &empty); err != nil {
		t.Fatal(err)
	}
	pr = Item{Kind: KindPR}
	giteaClient().applyStatus(&pr, empty)
	if pr.CI != CINone || pr.Checks != nil {
		t.Fatalf("a commit with no statuses must have no CI: %q / %+v", pr.CI, pr.Checks)
	}
}

func TestGiteaStates(t *testing.T) {
	for in, want := range map[string]CIState{
		"success": CISuccess,
		"warning": CISuccess,
		"failure": CIFailure,
		"error":   CIFailure,
		"skipped": CICanceled,
		"pending": CIPending,
		"":        CINone,
	} {
		if got := gtState(in); got != want {
			t.Errorf("gtState(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestGiteaReviewState(t *testing.T) {
	for _, tt := range []struct {
		review gtReview
		want   string
	}{
		{gtReview{State: "APPROVED"}, ReviewApproved},
		{gtReview{State: "REQUEST_CHANGES"}, ReviewChangesRequested},
		{gtReview{State: "COMMENT"}, "commented"},
		{gtReview{State: "APPROVED", Dismissed: true}, "dismissed"},
	} {
		if got := gtReviewState(tt.review); got != tt.want {
			t.Errorf("gtReviewState(%+v) = %q, want %q", tt.review, got, tt.want)
		}
	}
}

// tea exits 0 even when the server answers 4xx, so the envelope is the only
// signal that a request failed.
func TestGiteaAPIError(t *testing.T) {
	err := apiError([]byte(`{"message":"token is required","url":"https://git.corp/api/swagger"}`))
	if err == nil || err.Error() != "token is required" {
		t.Fatalf("got %v", err)
	}
	var apiErr *giteaAPIError
	if !errors.As(err, &apiErr) {
		t.Fatal("the envelope must stay recognizable so the auth hint can be added")
	}
	for _, body := range []string{``, `[]`, `[{"number":1}]`, `{"login":"ana"}`, `null`} {
		if err := apiError([]byte(body)); err != nil {
			t.Errorf("apiError(%q) = %v, want nil", body, err)
		}
	}
}

func TestPickLogin(t *testing.T) {
	logins := []teaLogin{
		{Name: "pessoal", URL: "https://git.corp", User: "ana"},
		{Name: "corp", URL: "https://git.corp/", User: "ana.silva", Default: "true"},
		{Name: "outro", URL: "https://git.outra.org", User: "ana"},
	}
	// An instance named after a login pins that account.
	if l, ok := pickLogin(logins, "git.corp", "pessoal"); !ok || l.Name != "pessoal" {
		t.Fatalf("got %+v, %v", l, ok)
	}
	// Otherwise the default login for the host wins, trailing slash and all.
	if l, ok := pickLogin(logins, "git.corp", "sem-nome"); !ok || l.Name != "corp" {
		t.Fatalf("got %+v, %v", l, ok)
	}
	if l, ok := pickLogin(logins, "https://git.outra.org", "x"); !ok || l.Name != "outro" {
		t.Fatalf("got %+v, %v", l, ok)
	}
	if _, ok := pickLogin(logins, "git.desconhecido", "x"); ok {
		t.Fatal("a host tea does not know must not match a login")
	}
}

func TestTeaVersion(t *testing.T) {
	// tea styles the version, so the escape sequences have to be tolerated.
	major, minor, ok := teaVersion([]byte("Version: \x1b[1m0.15.1\x1b[0m\tgolang: 1.26.5\tgo-sdk: v1.2.0"))
	if !ok || major != 0 || minor != 15 {
		t.Fatalf("got %d.%d, ok = %v", major, minor, ok)
	}
	if major, minor, ok := teaVersion([]byte("Version: 1.2.3")); !ok || major != 1 || minor != 2 {
		t.Fatalf("got %d.%d, ok = %v", major, minor, ok)
	}
	// Another program called tea must not be mistaken for the Gitea CLI.
	if _, _, ok := teaVersion([]byte("tea 1.0.0\nthe package manager")); ok {
		t.Fatal("a foreign tea binary must not parse as the Gitea CLI")
	}
}

func TestGiteaAbsolute(t *testing.T) {
	g := giteaClient()
	if got := g.absolute("/acme/app/actions/runs/1"); got != "https://git.corp/acme/app/actions/runs/1" {
		t.Fatalf("got %q", got)
	}
	if got := g.absolute("https://ci.corp/build/1"); got != "https://ci.corp/build/1" {
		t.Fatalf("absolute URLs must be left alone: %q", got)
	}
	// Without a resolved login the host from the configuration is the fallback.
	plain := &gitea{in: config.Instance{Host: "git.corp"}}
	if got := plain.serverURL(); got != "https://git.corp" {
		t.Fatalf("got %q", got)
	}
}
