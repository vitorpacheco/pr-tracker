package provider

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

type gitlab struct {
	in      config.Instance
	tracked []string
}

func (g *gitlab) Instance() config.Instance { return g.in }
func (g *gitlab) Tool() string              { return "glab" }

func (g *gitlab) env() []string { return []string{"GITLAB_HOST=" + g.in.Host} }

// repoArg is the -R value accepted by glab; the full URL pins the host.
func (g *gitlab) repoArg(pr *Item) string {
	if pr.RepoURL != "" {
		return pr.RepoURL
	}
	return "https://" + g.in.Host + "/" + pr.Repo
}

const glQuery = `
query {
  currentUser {
    username
    review: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    authored: authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
  }
}
# Labels/assignees are left out: GitLab caps query complexity at 200-250.
fragment mr on MergeRequest {
  iid title webUrl draft createdAt updatedAt userNotesCount
  sourceBranch targetBranch conflicts approved
  diffStatsSummary { additions deletions fileCount }
  author { username }
  project { fullPath webUrl }
  sourceProject { fullPath }
  approvedBy { nodes { username } }
  headPipeline {
    status
    jobs(first: 50) { nodes { name status webPath } }
  }
}`

const glTrackedQuery = `
query($repo: ID!, $cursor: String) {
  project(fullPath: $repo) {
    mergeRequests(state: opened, first: 50, after: $cursor, sort: UPDATED_DESC) {
      nodes { ...mr }
      pageInfo { hasNextPage endCursor }
    }
  }
}
# Labels/assignees are left out: GitLab caps query complexity at 200-250.
fragment mr on MergeRequest {
  iid title webUrl draft createdAt updatedAt userNotesCount
  sourceBranch targetBranch conflicts approved
  diffStatsSummary { additions deletions fileCount }
  author { username }
  project { fullPath webUrl }
  sourceProject { fullPath }
  approvedBy { nodes { username } }
  headPipeline {
    status
    jobs(first: 50) { nodes { name status webPath } }
  }
}`

type glMR struct {
	IID          string    `json:"iid"`
	Title        string    `json:"title"`
	WebURL       string    `json:"webUrl"`
	Draft        bool      `json:"draft"`
	CreatedAt    time.Time `json:"createdAt"`
	UpdatedAt    time.Time `json:"updatedAt"`
	SourceBranch string    `json:"sourceBranch"`
	TargetBranch string    `json:"targetBranch"`
	Conflicts    bool      `json:"conflicts"`
	Approved     bool      `json:"approved"`
	Notes        int       `json:"userNotesCount"`
	DiffStats    *struct {
		Additions int `json:"additions"`
		Deletions int `json:"deletions"`
		FileCount int `json:"fileCount"`
	} `json:"diffStatsSummary"`
	Author *struct {
		Username string `json:"username"`
	} `json:"author"`
	Project struct {
		FullPath string `json:"fullPath"`
		WebURL   string `json:"webUrl"`
	} `json:"project"`
	SourceProject *struct {
		FullPath string `json:"fullPath"`
	} `json:"sourceProject"`
	ApprovedBy struct {
		Nodes []struct {
			Username string `json:"username"`
		} `json:"nodes"`
	} `json:"approvedBy"`
	HeadPipeline *struct {
		Status string `json:"status"`
		Jobs   struct {
			Nodes []struct {
				Name    string `json:"name"`
				Status  string `json:"status"`
				WebPath string `json:"webPath"`
			} `json:"nodes"`
		} `json:"jobs"`
	} `json:"headPipeline"`
}

type glUser struct {
	Username string `json:"username"`
}

type glUsers struct {
	Nodes []glUser `json:"nodes"`
}

type glLabels struct {
	Nodes []struct {
		Title string `json:"title"`
	} `json:"nodes"`
}

func (l glLabels) names() []string {
	var out []string
	for _, n := range l.Nodes {
		out = append(out, n.Title)
	}
	return out
}

func (u glUsers) names() []string {
	var out []string
	for _, n := range u.Nodes {
		out = append(out, n.Username)
	}
	return out
}

type glConn struct {
	Nodes    []glMR `json:"nodes"`
	PageInfo struct {
		HasNextPage bool   `json:"hasNextPage"`
		EndCursor   string `json:"endCursor"`
	} `json:"pageInfo"`
}

type glResponse struct {
	Data struct {
		CurrentUser *struct {
			Username string `json:"username"`
			Review   glConn `json:"review"`
			Authored glConn `json:"authored"`
			Assigned glConn `json:"assigned"`
		} `json:"currentUser"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

type glTrackedResponse struct {
	Data struct {
		Project *struct {
			MergeRequests glConn `json:"mergeRequests"`
		} `json:"project"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

func (g *gitlab) List(ctx context.Context) ([]Item, error) {
	out, err := run(ctx, "", g.env(), "glab", "api", "graphql", "--hostname", g.in.Host, "-f", "query="+glQuery)
	if err != nil {
		return nil, err
	}
	var resp glResponse
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, fmt.Errorf("resposta inválida do glab: %w", err)
	}
	if resp.Data.CurrentUser == nil {
		if len(resp.Errors) > 0 {
			return nil, fmt.Errorf("glab graphql: %s", resp.Errors[0].Message)
		}
		return nil, fmt.Errorf("glab não autenticado em %s. Rode: glab auth login --hostname %s", g.in.Host, g.in.Host)
	}
	u := resp.Data.CurrentUser
	acc := newAccumulator()
	for _, s := range []struct {
		rel   Relation
		nodes []glMR
	}{
		{ReviewRequested, u.Review.Nodes},
		{Authored, u.Authored.Nodes},
		{Assigned, u.Assigned.Nodes},
	} {
		for _, n := range s.nodes {
			acc.add(g.convert(n, u.Username), s.rel)
		}
	}
	for _, repo := range g.tracked {
		cursor := ""
		for {
			args := []string{"api", "graphql", "--hostname", g.in.Host,
				"-f", "query=" + glTrackedQuery, "-f", "repo=" + repo}
			if cursor != "" {
				args = append(args, "-f", "cursor="+cursor)
			}
			out, err := run(ctx, "", g.env(), "glab", args...)
			if err != nil {
				return nil, err
			}
			var tracked glTrackedResponse
			if err := json.Unmarshal(out, &tracked); err != nil {
				return nil, fmt.Errorf("resposta inválida do glab para %s: %w", repo, err)
			}
			if len(tracked.Errors) > 0 {
				return nil, fmt.Errorf("glab graphql (%s): %s", repo, tracked.Errors[0].Message)
			}
			if tracked.Data.Project == nil {
				return nil, fmt.Errorf("projeto %s não encontrado em %s", repo, g.in.Host)
			}
			mrs := tracked.Data.Project.MergeRequests
			for _, n := range mrs.Nodes {
				acc.add(g.convert(n, u.Username), 0)
			}
			if !mrs.PageInfo.HasNextPage || mrs.PageInfo.EndCursor == "" || mrs.PageInfo.EndCursor == cursor {
				break
			}
			cursor = mrs.PageInfo.EndCursor
		}
	}
	if err := g.listIssues(ctx, u.Username, acc); err != nil {
		return nil, err
	}
	return acc.list(), nil
}

const glIssuesQuery = `
query($me: String!) {
  assigned: issues(assigneeUsernames: [$me], state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...issue } }
  authored: issues(authorUsername: $me, state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...issue } }
  currentUser {
    todos(action: [mentioned, directly_addressed], type: [ISSUE, WORKITEM], state: [pending], first: 50) {
      nodes {
        project { webUrl }
        target {
          ... on Issue { ...issue }
          ... on WorkItem { iid title webUrl createdAt updatedAt state reference(full: true) author { username } }
        }
      }
    }
  }
}
fragment issue on Issue {
  iid title webUrl createdAt updatedAt state reference(full: true) userNotesCount
  author { username }
  labels(first: 10) { nodes { title } }
  assignees(first: 10) { nodes { username } }
}`

type glIssue struct {
	IID       string    `json:"iid"`
	Title     string    `json:"title"`
	WebURL    string    `json:"webUrl"`
	CreatedAt time.Time `json:"createdAt"`
	UpdatedAt time.Time `json:"updatedAt"`
	State     string    `json:"state"`
	Reference string    `json:"reference"`
	Notes     int       `json:"userNotesCount"`
	Author    *glUser   `json:"author"`
	Labels    glLabels  `json:"labels"`
	Assignees glUsers   `json:"assignees"`
}

// listIssues adds open issues assigned to, authored by or mentioning the user.
func (g *gitlab) listIssues(ctx context.Context, me string, acc *accumulator) error {
	out, err := run(ctx, "", g.env(), "glab", "api", "graphql", "--hostname", g.in.Host,
		"-f", "query="+glIssuesQuery, "-f", "me="+me)
	if err != nil {
		return err
	}
	var resp struct {
		Data struct {
			Assigned    struct{ Nodes []glIssue } `json:"assigned"`
			Authored    struct{ Nodes []glIssue } `json:"authored"`
			CurrentUser *struct {
				Todos struct {
					Nodes []struct {
						Project *struct {
							WebURL string `json:"webUrl"`
						} `json:"project"`
						Target glIssue `json:"target"`
					} `json:"nodes"`
				} `json:"todos"`
			} `json:"currentUser"`
		} `json:"data"`
		Errors []struct {
			Message string `json:"message"`
		} `json:"errors"`
	}
	if err := json.Unmarshal(out, &resp); err != nil {
		return fmt.Errorf("resposta inválida do glab: %w", err)
	}
	if len(resp.Errors) > 0 && resp.Data.CurrentUser == nil {
		return fmt.Errorf("glab graphql (issues): %s", resp.Errors[0].Message)
	}
	for _, n := range resp.Data.Assigned.Nodes {
		acc.add(g.convertIssue(n), Assigned)
	}
	for _, n := range resp.Data.Authored.Nodes {
		acc.add(g.convertIssue(n), Authored)
	}
	if cu := resp.Data.CurrentUser; cu != nil {
		for _, t := range cu.Todos.Nodes {
			if t.Target.IID == "" || t.Target.State != "opened" {
				continue
			}
			acc.add(g.convertIssue(t.Target), Mentioned)
		}
	}
	return nil
}

func (g *gitlab) convertIssue(n glIssue) Item {
	iid, _ := strconv.Atoi(n.IID)
	repo := n.Reference
	if i := strings.LastIndex(repo, "#"); i > 0 {
		repo = repo[:i]
	}
	repoURL, _, _ := strings.Cut(n.WebURL, "/-/")
	it := Item{
		Kind:      KindIssue,
		Instance:  g.in.Name,
		Provider:  config.GitLab,
		Host:      g.in.Host,
		Repo:      repo,
		RepoURL:   repoURL,
		Number:    iid,
		Title:     n.Title,
		URL:       n.WebURL,
		CreatedAt: n.CreatedAt,
		UpdatedAt: n.UpdatedAt,
		Comments:  n.Notes,
		Labels:    n.Labels.names(),
		Assignees: n.Assignees.names(),
	}
	if n.Author != nil {
		it.Author = n.Author.Username
	}
	return it
}

const glThreadQuery = `
query($path: ID!, $iid: String!) {
  project(fullPath: $path) {
    %s(iid: $iid) {
      description
      notes(filter: ONLY_COMMENTS, last: 100) { nodes {
        body createdAt system
        author { username }
        position { filePath newLine oldLine }
      } }
    }
  }
}`

func (g *gitlab) Thread(ctx context.Context, it *Item) (*Thread, error) {
	field := "mergeRequest"
	if it.IsIssue() {
		field = "issue"
	}
	out, err := run(ctx, "", g.env(), "glab", "api", "graphql", "--hostname", g.in.Host,
		"-f", "query="+fmt.Sprintf(glThreadQuery, field), "-f", "path="+it.Repo, "-f", "iid="+strconv.Itoa(it.Number))
	if err != nil {
		return nil, err
	}
	type note struct {
		Body      string    `json:"body"`
		CreatedAt time.Time `json:"createdAt"`
		System    bool      `json:"system"`
		Author    *glUser   `json:"author"`
		Position  *struct {
			FilePath string `json:"filePath"`
			NewLine  int    `json:"newLine"`
			OldLine  int    `json:"oldLine"`
		} `json:"position"`
	}
	var resp struct {
		Data struct {
			Project *struct {
				MergeRequest *struct {
					Description string `json:"description"`
					Notes       struct {
						Nodes []note `json:"nodes"`
					} `json:"notes"`
				} `json:"mergeRequest"`
				Issue *struct {
					Description string `json:"description"`
					Notes       struct {
						Nodes []note `json:"nodes"`
					} `json:"notes"`
				} `json:"issue"`
			} `json:"project"`
		} `json:"data"`
		Errors []struct {
			Message string `json:"message"`
		} `json:"errors"`
	}
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, fmt.Errorf("resposta inválida do glab: %w", err)
	}
	var desc string
	var notes []note
	switch p := resp.Data.Project; {
	case p != nil && p.MergeRequest != nil:
		desc, notes = p.MergeRequest.Description, p.MergeRequest.Notes.Nodes
	case p != nil && p.Issue != nil:
		desc, notes = p.Issue.Description, p.Issue.Notes.Nodes
	case len(resp.Errors) > 0:
		return nil, fmt.Errorf("glab graphql: %s", resp.Errors[0].Message)
	default:
		return nil, fmt.Errorf("%s%s não encontrado", it.Repo, it.Ref())
	}
	t := &Thread{Body: desc}
	for _, n := range notes {
		if n.System {
			continue
		}
		c := Comment{Body: n.Body, CreatedAt: n.CreatedAt}
		if n.Author != nil {
			c.Author = n.Author.Username
		}
		if n.Position != nil {
			c.Path, c.Line = n.Position.FilePath, n.Position.NewLine
			if c.Line == 0 {
				c.Line = n.Position.OldLine
			}
		}
		t.Comments = append(t.Comments, c)
	}
	sortComments(t.Comments)
	return t, nil
}

func (g *gitlab) AddComment(ctx context.Context, it *Item, body string) error {
	kind := "merge_requests"
	if it.IsIssue() {
		kind = "issues"
	}
	endpoint := fmt.Sprintf("projects/%s/%s/%d/notes", url.PathEscape(it.Repo), kind, it.Number)
	_, err := run(ctx, "", g.env(), "glab", "api", "--hostname", g.in.Host, "--method", "POST", endpoint, "-f", "body="+body)
	return err
}

func (g *gitlab) convert(n glMR, me string) Item {
	iid, _ := strconv.Atoi(n.IID)
	pr := Item{
		Instance:     g.in.Name,
		Provider:     config.GitLab,
		Host:         g.in.Host,
		Repo:         n.Project.FullPath,
		RepoURL:      n.Project.WebURL,
		Number:       iid,
		Title:        n.Title,
		URL:          n.WebURL,
		Draft:        n.Draft,
		SourceBranch: n.SourceBranch,
		TargetBranch: n.TargetBranch,
		CreatedAt:    n.CreatedAt,
		UpdatedAt:    n.UpdatedAt,
		Conflicts:    n.Conflicts,
		Comments:     n.Notes,
	}
	if n.SourceProject != nil && n.SourceProject.FullPath != n.Project.FullPath {
		pr.FromFork = true
	}
	if n.Author != nil {
		pr.Author = n.Author.Username
	}
	if n.DiffStats != nil {
		pr.Additions, pr.Deletions, pr.Files = n.DiffStats.Additions, n.DiffStats.Deletions, n.DiffStats.FileCount
	}
	for _, a := range n.ApprovedBy.Nodes {
		pr.ApprovedBy = append(pr.ApprovedBy, a.Username)
		if strings.EqualFold(a.Username, me) {
			pr.ApprovedByMe = true
		}
	}
	if n.Approved {
		pr.Review = ReviewApproved
	} else {
		pr.Review = ReviewRequired
	}
	if p := n.HeadPipeline; p != nil {
		pr.CI = glState(p.Status)
		for _, j := range p.Jobs.Nodes {
			url := ""
			if j.WebPath != "" {
				url = "https://" + g.in.Host + j.WebPath
			}
			pr.Checks = append(pr.Checks, Check{Name: j.Name, State: glState(j.Status), URL: url})
		}
	}
	return pr
}

func glState(s string) CIState {
	switch strings.ToUpper(s) {
	case "SUCCESS":
		return CISuccess
	case "FAILED":
		return CIFailure
	case "CANCELED", "CANCELING", "SKIPPED":
		return CICanceled
	case "CREATED", "WAITING_FOR_RESOURCE", "PREPARING", "PENDING", "RUNNING", "SCHEDULED", "MANUAL", "WAITING_FOR_CALLBACK":
		return CIPending
	}
	return CINone
}

func (g *gitlab) Approve(ctx context.Context, pr *Item) error {
	_, err := run(ctx, "", g.env(), "glab", "mr", "approve", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *gitlab) Merge(ctx context.Context, pr *Item, opts MergeOptions) error {
	args := []string{"mr", "merge", strconv.Itoa(pr.Number), "-R", g.repoArg(pr), "--yes",
		"--auto-merge=" + strconv.FormatBool(opts.Auto)}
	switch mergeMethod(opts.Method) {
	case "squash":
		args = append(args, "--squash")
	case "rebase":
		args = append(args, "--rebase")
	}
	if opts.DeleteBranch {
		args = append(args, "--remove-source-branch")
	}
	_, err := run(ctx, "", g.env(), "glab", args...)
	return err
}

func (g *gitlab) Close(ctx context.Context, pr *Item) error {
	_, err := run(ctx, "", g.env(), "glab", "mr", "close", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *gitlab) Checkout(ctx context.Context, pr *Item, dir string) error {
	_, err := run(ctx, dir, g.env(), "glab", "mr", "checkout", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *gitlab) HeadRef(pr *Item) string {
	return fmt.Sprintf("refs/merge-requests/%d/head", pr.Number)
}

func (g *gitlab) AuthStatus(ctx context.Context) error {
	_, err := run(ctx, "", nil, "glab", "auth", "status", "--hostname", g.in.Host)
	if err != nil && !isMissing(err) {
		return fmt.Errorf("glab não autenticado em %s. Rode: glab auth login --hostname %s", g.in.Host, g.in.Host)
	}
	return err
}
