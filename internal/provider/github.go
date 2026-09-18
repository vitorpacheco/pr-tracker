package provider

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

type github struct{ in config.Instance }

func (g *github) Instance() config.Instance { return g.in }
func (g *github) Tool() string              { return "gh" }

func (g *github) env() []string { return []string{"GH_HOST=" + g.in.Host} }

// repoArg is the -R value accepted by gh: HOST/OWNER/REPO.
func (g *github) repoArg(pr *PR) string { return g.in.Host + "/" + pr.Repo }

const ghQuery = `
query($review: String!, $authored: String!, $assigned: String!) {
  viewer { login }
  review: search(query: $review, type: ISSUE, first: 50) { nodes { ...pr } }
  authored: search(query: $authored, type: ISSUE, first: 50) { nodes { ...pr } }
  assigned: search(query: $assigned, type: ISSUE, first: 50) { nodes { ...pr } }
}
fragment pr on PullRequest {
  number title url isDraft createdAt updatedAt
  headRefName baseRefName isCrossRepository
  reviewDecision mergeable additions deletions changedFiles
  author { login }
  repository { nameWithOwner url }
  latestReviews(first: 30) { nodes { state author { login } } }
  commits(last: 1) { nodes { commit { statusCheckRollup {
    state
    contexts(first: 50) { nodes {
      __typename
      ... on CheckRun { name status conclusion detailsUrl }
      ... on StatusContext { context state targetUrl }
    } }
  } } } }
}`

type ghPR struct {
	Number            int       `json:"number"`
	Title             string    `json:"title"`
	URL               string    `json:"url"`
	IsDraft           bool      `json:"isDraft"`
	CreatedAt         time.Time `json:"createdAt"`
	UpdatedAt         time.Time `json:"updatedAt"`
	HeadRefName       string    `json:"headRefName"`
	BaseRefName       string    `json:"baseRefName"`
	IsCrossRepository bool      `json:"isCrossRepository"`
	ReviewDecision    string    `json:"reviewDecision"`
	Mergeable         string    `json:"mergeable"`
	Additions         int       `json:"additions"`
	Deletions         int       `json:"deletions"`
	ChangedFiles      int       `json:"changedFiles"`
	Author            *struct {
		Login string `json:"login"`
	} `json:"author"`
	Repository struct {
		NameWithOwner string `json:"nameWithOwner"`
		URL           string `json:"url"`
	} `json:"repository"`
	LatestReviews struct {
		Nodes []struct {
			State  string `json:"state"`
			Author *struct {
				Login string `json:"login"`
			} `json:"author"`
		} `json:"nodes"`
	} `json:"latestReviews"`
	Commits struct {
		Nodes []struct {
			Commit struct {
				StatusCheckRollup *struct {
					State    string `json:"state"`
					Contexts struct {
						Nodes []struct {
							Typename   string `json:"__typename"`
							Name       string `json:"name"`
							Status     string `json:"status"`
							Conclusion string `json:"conclusion"`
							DetailsURL string `json:"detailsUrl"`
							Context    string `json:"context"`
							State      string `json:"state"`
							TargetURL  string `json:"targetUrl"`
						} `json:"nodes"`
					} `json:"contexts"`
				} `json:"statusCheckRollup"`
			} `json:"commit"`
		} `json:"nodes"`
	} `json:"commits"`
}

type ghSearch struct {
	Nodes []ghPR `json:"nodes"`
}

type ghResponse struct {
	Data struct {
		Viewer struct {
			Login string `json:"login"`
		} `json:"viewer"`
		Review   ghSearch `json:"review"`
		Authored ghSearch `json:"authored"`
		Assigned ghSearch `json:"assigned"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

func (g *github) List(ctx context.Context) ([]PR, error) {
	base := "is:pr is:open archived:false sort:updated-desc "
	out, err := run(ctx, "", g.env(), "gh", "api", "graphql", "--hostname", g.in.Host,
		"-f", "query="+ghQuery,
		"-f", "review="+base+"review-requested:@me",
		"-f", "authored="+base+"author:@me",
		"-f", "assigned="+base+"assignee:@me",
	)
	if err != nil {
		return nil, err
	}
	var resp ghResponse
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, fmt.Errorf("resposta inválida do gh: %w", err)
	}
	if len(resp.Errors) > 0 && len(resp.Data.Review.Nodes)+len(resp.Data.Authored.Nodes)+len(resp.Data.Assigned.Nodes) == 0 {
		return nil, fmt.Errorf("gh graphql: %s", resp.Errors[0].Message)
	}
	me := resp.Data.Viewer.Login
	acc := newAccumulator()
	for _, s := range []struct {
		rel   Relation
		nodes []ghPR
	}{
		{ReviewRequested, resp.Data.Review.Nodes},
		{Authored, resp.Data.Authored.Nodes},
		{Assigned, resp.Data.Assigned.Nodes},
	} {
		for _, n := range s.nodes {
			if n.Number == 0 { // search can return non-PR nodes as empty objects
				continue
			}
			acc.add(g.convert(n, me), s.rel)
		}
	}
	return acc.list(), nil
}

func (g *github) convert(n ghPR, me string) PR {
	pr := PR{
		Instance:     g.in.Name,
		Provider:     config.GitHub,
		Host:         g.in.Host,
		Repo:         n.Repository.NameWithOwner,
		RepoURL:      n.Repository.URL,
		Number:       n.Number,
		Title:        n.Title,
		URL:          n.URL,
		Draft:        n.IsDraft,
		SourceBranch: n.HeadRefName,
		TargetBranch: n.BaseRefName,
		FromFork:     n.IsCrossRepository,
		CreatedAt:    n.CreatedAt,
		UpdatedAt:    n.UpdatedAt,
		Additions:    n.Additions,
		Deletions:    n.Deletions,
		Files:        n.ChangedFiles,
		Conflicts:    n.Mergeable == "CONFLICTING",
	}
	if n.Author != nil {
		pr.Author = n.Author.Login
	}
	switch n.ReviewDecision {
	case "APPROVED":
		pr.Review = ReviewApproved
	case "CHANGES_REQUESTED":
		pr.Review = ReviewChangesRequested
	case "REVIEW_REQUIRED":
		pr.Review = ReviewRequired
	}
	for _, r := range n.LatestReviews.Nodes {
		if r.State != "APPROVED" || r.Author == nil {
			continue
		}
		pr.ApprovedBy = append(pr.ApprovedBy, r.Author.Login)
		if strings.EqualFold(r.Author.Login, me) {
			pr.ApprovedByMe = true
		}
	}
	if len(n.Commits.Nodes) > 0 {
		if roll := n.Commits.Nodes[0].Commit.StatusCheckRollup; roll != nil {
			pr.CI = ghState(roll.State)
			for _, c := range roll.Contexts.Nodes {
				if c.Typename == "CheckRun" {
					st := CIPending
					if c.Status == "COMPLETED" {
						st = ghState(c.Conclusion)
					}
					pr.Checks = append(pr.Checks, Check{Name: c.Name, State: st, URL: c.DetailsURL})
				} else {
					pr.Checks = append(pr.Checks, Check{Name: c.Context, State: ghState(c.State), URL: c.TargetURL})
				}
			}
		}
	}
	return pr
}

func ghState(s string) CIState {
	switch s {
	case "SUCCESS", "NEUTRAL":
		return CISuccess
	case "FAILURE", "ERROR", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE":
		return CIFailure
	case "CANCELLED", "SKIPPED", "STALE":
		return CICanceled
	case "PENDING", "EXPECTED", "QUEUED", "IN_PROGRESS", "WAITING", "REQUESTED":
		return CIPending
	}
	return CINone
}

func (g *github) Approve(ctx context.Context, pr *PR) error {
	_, err := run(ctx, "", g.env(), "gh", "pr", "review", strconv.Itoa(pr.Number), "--approve", "-R", g.repoArg(pr))
	return err
}

func (g *github) Merge(ctx context.Context, pr *PR, opts MergeOptions) error {
	args := []string{"pr", "merge", strconv.Itoa(pr.Number), "-R", g.repoArg(pr), "--" + mergeMethod(opts.Method)}
	if opts.Auto {
		args = append(args, "--auto")
	}
	if opts.DeleteBranch {
		args = append(args, "--delete-branch")
	}
	_, err := run(ctx, "", g.env(), "gh", args...)
	return err
}

func (g *github) Checkout(ctx context.Context, pr *PR, dir string) error {
	_, err := run(ctx, dir, g.env(), "gh", "pr", "checkout", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *github) HeadRef(pr *PR) string { return fmt.Sprintf("refs/pull/%d/head", pr.Number) }

func (g *github) AuthStatus(ctx context.Context) error {
	_, err := run(ctx, "", nil, "gh", "auth", "status", "--hostname", g.in.Host)
	if err != nil && !isMissing(err) {
		return fmt.Errorf("gh não autenticado em %s. Rode: gh auth login --hostname %s", g.in.Host, g.in.Host)
	}
	return err
}
