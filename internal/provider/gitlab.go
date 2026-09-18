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

type gitlab struct{ in config.Instance }

func (g *gitlab) Instance() config.Instance { return g.in }
func (g *gitlab) Tool() string              { return "glab" }

func (g *gitlab) env() []string { return []string{"GITLAB_HOST=" + g.in.Host} }

// repoArg is the -R value accepted by glab; the full URL pins the host.
func (g *gitlab) repoArg(pr *PR) string {
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
fragment mr on MergeRequest {
  iid title webUrl draft createdAt updatedAt
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

type glConn struct {
	Nodes []glMR `json:"nodes"`
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

func (g *gitlab) List(ctx context.Context) ([]PR, error) {
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
	return acc.list(), nil
}

func (g *gitlab) convert(n glMR, me string) PR {
	iid, _ := strconv.Atoi(n.IID)
	pr := PR{
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

func (g *gitlab) Approve(ctx context.Context, pr *PR) error {
	_, err := run(ctx, "", g.env(), "glab", "mr", "approve", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *gitlab) Merge(ctx context.Context, pr *PR, opts MergeOptions) error {
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

func (g *gitlab) Checkout(ctx context.Context, pr *PR, dir string) error {
	_, err := run(ctx, dir, g.env(), "glab", "mr", "checkout", strconv.Itoa(pr.Number), "-R", g.repoArg(pr))
	return err
}

func (g *gitlab) HeadRef(pr *PR) string { return fmt.Sprintf("refs/merge-requests/%d/head", pr.Number) }

func (g *gitlab) AuthStatus(ctx context.Context) error {
	_, err := run(ctx, "", nil, "glab", "auth", "status", "--hostname", g.in.Host)
	if err != nil && !isMissing(err) {
		return fmt.Errorf("glab não autenticado em %s. Rode: glab auth login --hostname %s", g.in.Host, g.in.Host)
	}
	return err
}
