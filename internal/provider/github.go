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
func (g *github) repoArg(pr *Item) string { return g.in.Host + "/" + pr.Repo }

const ghQuery = `
query($review: String!, $authored: String!, $assigned: String!,
      $issAssigned: String!, $issAuthored: String!, $issMentioned: String!) {
  viewer { login }
  review: search(query: $review, type: ISSUE, first: 50) { nodes { ...pr } }
  authored: search(query: $authored, type: ISSUE, first: 50) { nodes { ...pr } }
  assigned: search(query: $assigned, type: ISSUE, first: 50) { nodes { ...pr } }
  issAssigned: search(query: $issAssigned, type: ISSUE, first: 50) { nodes { ...issue } }
  issAuthored: search(query: $issAuthored, type: ISSUE, first: 50) { nodes { ...issue } }
  issMentioned: search(query: $issMentioned, type: ISSUE, first: 50) { nodes { ...issue } }
}
fragment issue on Issue {
  number title url createdAt updatedAt
  author { login }
  repository { nameWithOwner url }
  comments { totalCount }
  labels(first: 10) { nodes { name } }
  assignees(first: 10) { nodes { login } }
}
fragment pr on PullRequest {
  number title url isDraft createdAt updatedAt
  comments { totalCount }
  labels(first: 10) { nodes { name } }
  assignees(first: 10) { nodes { login } }
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
	Comments          struct {
		TotalCount int `json:"totalCount"`
	} `json:"comments"`
	Labels struct {
		Nodes []struct {
			Name string `json:"name"`
		} `json:"nodes"`
	} `json:"labels"`
	Assignees struct {
		Nodes []ghLogin `json:"nodes"`
	} `json:"assignees"`
	Author     *ghLogin `json:"author"`
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

type ghLogin struct {
	Login string `json:"login"`
}

type ghSearch struct {
	Nodes []ghPR `json:"nodes"`
}

type ghResponse struct {
	Data struct {
		Viewer struct {
			Login string `json:"login"`
		} `json:"viewer"`
		Review       ghSearch `json:"review"`
		Authored     ghSearch `json:"authored"`
		Assigned     ghSearch `json:"assigned"`
		IssAssigned  ghSearch `json:"issAssigned"`
		IssAuthored  ghSearch `json:"issAuthored"`
		IssMentioned ghSearch `json:"issMentioned"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

func (g *github) List(ctx context.Context) ([]Item, error) {
	base := "is:pr is:open archived:false sort:updated-desc "
	iss := "is:issue is:open archived:false sort:updated-desc "
	out, err := run(ctx, "", g.env(), "gh", "api", "graphql", "--hostname", g.in.Host,
		"-f", "query="+ghQuery,
		"-f", "review="+base+"review-requested:@me",
		"-f", "authored="+base+"author:@me",
		"-f", "assigned="+base+"assignee:@me",
		"-f", "issAssigned="+iss+"assignee:@me",
		"-f", "issAuthored="+iss+"author:@me",
		"-f", "issMentioned="+iss+"mentions:@me",
	)
	if err != nil {
		return nil, err
	}
	var resp ghResponse
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, fmt.Errorf("resposta inválida do gh: %w", err)
	}
	if len(resp.Errors) > 0 && resp.Data.Viewer.Login == "" {
		return nil, fmt.Errorf("gh graphql: %s", resp.Errors[0].Message)
	}
	me := resp.Data.Viewer.Login
	acc := newAccumulator()
	for _, s := range []struct {
		kind  Kind
		rel   Relation
		nodes []ghPR
	}{
		{KindPR, ReviewRequested, resp.Data.Review.Nodes},
		{KindPR, Authored, resp.Data.Authored.Nodes},
		{KindPR, Assigned, resp.Data.Assigned.Nodes},
		{KindIssue, Assigned, resp.Data.IssAssigned.Nodes},
		{KindIssue, Authored, resp.Data.IssAuthored.Nodes},
		{KindIssue, Mentioned, resp.Data.IssMentioned.Nodes},
	} {
		for _, n := range s.nodes {
			if n.Number == 0 { // search can return other node types as empty objects
				continue
			}
			it := g.convert(n, me)
			it.Kind = s.kind
			acc.add(it, s.rel)
		}
	}
	return acc.list(), nil
}

func (g *github) convert(n ghPR, me string) Item {
	pr := Item{
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
		Comments:     n.Comments.TotalCount,
	}
	if n.Author != nil {
		pr.Author = n.Author.Login
	}
	for _, l := range n.Labels.Nodes {
		pr.Labels = append(pr.Labels, l.Name)
	}
	for _, a := range n.Assignees.Nodes {
		pr.Assignees = append(pr.Assignees, a.Login)
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

const ghThreadQuery = `
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issueOrPullRequest(number: $number) {
      ... on Issue {
        body
        comments(last: 100) { nodes { ...comment } }
      }
      ... on PullRequest {
        body
        comments(last: 100) { nodes { ...comment } }
        reviews(last: 50) { nodes {
          state body createdAt author { login }
          comments(first: 50) { nodes { ...comment path line originalLine } }
        } }
      }
    }
  }
}
fragment comment on Comment { body createdAt author { login } }`

type ghComment struct {
	Body         string    `json:"body"`
	CreatedAt    time.Time `json:"createdAt"`
	Author       *ghLogin  `json:"author"`
	Path         string    `json:"path"`
	Line         int       `json:"line"`
	OriginalLine int       `json:"originalLine"`
}

func (c ghComment) convert() Comment {
	out := Comment{Body: c.Body, CreatedAt: c.CreatedAt, Path: c.Path, Line: c.Line}
	if out.Line == 0 {
		out.Line = c.OriginalLine
	}
	if c.Author != nil {
		out.Author = c.Author.Login
	}
	return out
}

func (g *github) Thread(ctx context.Context, it *Item) (*Thread, error) {
	owner, name, _ := strings.Cut(it.Repo, "/")
	out, err := run(ctx, "", g.env(), "gh", "api", "graphql", "--hostname", g.in.Host,
		"-f", "query="+ghThreadQuery, "-f", "owner="+owner, "-f", "name="+name,
		"-F", "number="+strconv.Itoa(it.Number))
	if err != nil {
		return nil, err
	}
	var resp struct {
		Data struct {
			Repository struct {
				Item *struct {
					Body     string `json:"body"`
					Comments struct {
						Nodes []ghComment `json:"nodes"`
					} `json:"comments"`
					Reviews struct {
						Nodes []struct {
							ghComment
							State    string `json:"state"`
							Comments struct {
								Nodes []ghComment `json:"nodes"`
							} `json:"comments"`
						} `json:"nodes"`
					} `json:"reviews"`
				} `json:"issueOrPullRequest"`
			} `json:"repository"`
		} `json:"data"`
		Errors []struct {
			Message string `json:"message"`
		} `json:"errors"`
	}
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, fmt.Errorf("resposta inválida do gh: %w", err)
	}
	src := resp.Data.Repository.Item
	if src == nil {
		if len(resp.Errors) > 0 {
			return nil, fmt.Errorf("gh graphql: %s", resp.Errors[0].Message)
		}
		return nil, fmt.Errorf("%s%s não encontrado", it.Repo, it.Ref())
	}
	t := &Thread{Body: src.Body}
	for _, c := range src.Comments.Nodes {
		t.Comments = append(t.Comments, c.convert())
	}
	for _, r := range src.Reviews.Nodes {
		// Reviews without a body and with no inline comments are just
		// "commented" markers left by inline replies; skip them.
		if r.Body != "" || r.State == "APPROVED" || r.State == "CHANGES_REQUESTED" {
			c := r.ghComment.convert()
			c.Review = strings.ToLower(r.State)
			t.Comments = append(t.Comments, c)
		}
		for _, rc := range r.Comments.Nodes {
			t.Comments = append(t.Comments, rc.convert())
		}
	}
	sortComments(t.Comments)
	return t, nil
}

func (g *github) AddComment(ctx context.Context, it *Item, body string) error {
	sub := "pr"
	if it.IsIssue() {
		sub = "issue"
	}
	_, err := run(ctx, "", g.env(), "gh", sub, "comment", strconv.Itoa(it.Number), "-R", g.repoArg(it), "--body", body)
	return err
}

func (g *github) Approve(ctx context.Context, pr *Item) error {
	_, err := run(ctx, "", g.env(), "gh", "pr", "review", strconv.Itoa(pr.Number), "--approve", "-R", g.repoArg(pr))
	return err
}

func (g *github) Merge(ctx context.Context, pr *Item, opts MergeOptions) error {
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

// Close leaves the branch alone: gh's --delete-branch would also delete the
// local branch of whatever clone the command runs in.
func (g *github) Close(ctx context.Context, pr *Item) error {
	_, err := run(ctx, "", g.env(), "gh", "pr", "close", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *github) Checkout(ctx context.Context, pr *Item, dir string) error {
	_, err := run(ctx, dir, g.env(), "gh", "pr", "checkout", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *github) HeadRef(pr *Item) string { return fmt.Sprintf("refs/pull/%d/head", pr.Number) }

func (g *github) AuthStatus(ctx context.Context) error {
	_, err := run(ctx, "", nil, "gh", "auth", "status", "--hostname", g.in.Host)
	if err != nil && !isMissing(err) {
		return fmt.Errorf("gh não autenticado em %s. Rode: gh auth login --hostname %s", g.in.Host, g.in.Host)
	}
	return err
}
