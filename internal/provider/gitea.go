package provider

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
)

// gitea talks to a Gitea server through tea (https://gitea.com/gitea/tea).
// Gitea has no GraphQL API, so the list is assembled from several REST calls:
// the issue search is the only endpoint that filters by how the current user
// relates to an item, but it answers with bare issues, so branches, diff stats,
// reviews and CI need one round of requests per pull request.
type gitea struct {
	in      config.Instance
	tracked []string

	mu    sync.Mutex
	login string // tea login name that serves this host
	base  string // server URL as tea stored it, without a trailing slash
}

func (g *gitea) Instance() config.Instance { return g.in }
func (g *gitea) Tool() string              { return "tea" }

// giteaWorkers bounds how many pull requests are enriched at once. Gitea
// servers are usually small and self-hosted; a refresh should not look like a
// burst of traffic.
const giteaWorkers = 6

// tea gained "tea api" in 0.12 and learned to accept a repository slug outside
// a clone in 0.14.1, which is what the pull commands here rely on.
const (
	teaMinMajor = 0
	teaMinMinor = 14
)

// ---------- login discovery ----------

// teaLogin is one entry of "tea logins list --output json". tea prints every
// value as a string, the default flag included.
type teaLogin struct {
	Name    string `json:"name"`
	URL     string `json:"url"`
	User    string `json:"user"`
	Default string `json:"default"`
}

// pickLogin chooses the tea login that serves host. tea addresses servers by
// login name, not by host like gh and glab do, so the mapping has to be
// discovered. An instance named after a login wins, which is how a user pins
// one of several accounts on the same host; otherwise the default login wins.
func pickLogin(logins []teaLogin, host, instance string) (teaLogin, bool) {
	var best teaLogin
	found := false
	for _, l := range logins {
		if !strings.EqualFold(config.NormalizeHost(l.URL), config.NormalizeHost(host)) {
			continue
		}
		if l.Name == instance {
			return l, true
		}
		if !found || l.Default == "true" {
			best, found = l, true
		}
	}
	return best, found
}

// The version is styled, so the pattern skips lazily over whatever escape
// sequences tea wrote around it.
var teaVersionRe = regexp.MustCompile(`Version:.*?([0-9]+)\.([0-9]+)\.([0-9]+)`)

// teaVersion parses the version out of "tea --version".
func teaVersion(out []byte) (major, minor int, ok bool) {
	m := teaVersionRe.FindSubmatch(out)
	if m == nil {
		return 0, 0, false
	}
	major, _ = strconv.Atoi(string(m[1]))
	minor, _ = strconv.Atoi(string(m[2]))
	return major, minor, true
}

// checkVersion also guards against a different program called tea: the name is
// generic enough to collide with unrelated tools on PATH.
func (g *gitea) checkVersion(ctx context.Context) error {
	out, err := run(ctx, "", nil, "tea", "--version")
	if err != nil {
		return err
	}
	major, minor, ok := teaVersion(out)
	if !ok {
		return i18n.Errorf("o tea encontrado no PATH não parece ser o CLI do Gitea: %s", strings.TrimSpace(string(out)))
	}
	if major < teaMinMajor || (major == teaMinMajor && minor < teaMinMinor) {
		return i18n.Errorf("tea %d.%d é antigo demais; a integração com Gitea precisa da %d.%d ou mais nova. Veja: %s",
			major, minor, teaMinMajor, teaMinMinor, InstallHint("tea"))
	}
	return nil
}

// resolve finds (once) the tea login for this instance.
func (g *gitea) resolve(ctx context.Context) (string, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.login != "" {
		return g.login, nil
	}
	if err := g.checkVersion(ctx); err != nil {
		return "", err
	}
	out, err := run(ctx, "", nil, "tea", "logins", "list", "--output", "json")
	if err != nil {
		return "", err
	}
	var logins []teaLogin
	if err := json.Unmarshal(out, &logins); err != nil {
		return "", i18n.Errorf("resposta inválida do tea: %w", err)
	}
	l, ok := pickLogin(logins, g.in.Host, g.in.Name)
	if !ok {
		return "", i18n.Errorf("tea não tem login para %s. Rode: tea login add --url https://%s", g.in.Host, g.in.Host)
	}
	g.login, g.base = l.Name, strings.TrimRight(l.URL, "/")
	return g.login, nil
}

// serverURL prefers the URL tea stored, so servers on plain HTTP or on a
// non-default port keep working.
func (g *gitea) serverURL() string {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.base != "" {
		return g.base
	}
	return "https://" + g.in.Host
}

// ---------- transport ----------

// giteaAPIError is the {"message": …} envelope Gitea answers errors with.
type giteaAPIError struct{ Message string }

func (e *giteaAPIError) Error() string { return e.Message }

// apiError inspects a response body. tea exits 0 even when the server answers
// 4xx, so the envelope is the only way to notice a failed request.
func apiError(out []byte) error {
	out = bytes.TrimSpace(out)
	if len(out) == 0 || out[0] != '{' {
		return nil
	}
	var e giteaAPIError
	if json.Unmarshal(out, &e) != nil || e.Message == "" {
		return nil
	}
	return &e
}

// api makes an authenticated request through tea. Flags go before the endpoint
// because tea stops parsing them at the first positional argument.
func (g *gitea) api(ctx context.Context, method, endpoint string, fields ...string) ([]byte, error) {
	login, err := g.resolve(ctx)
	if err != nil {
		return nil, err
	}
	args := []string{"api", "--login", login}
	if method != "" {
		args = append(args, "--method", method)
	}
	args = append(args, fields...)
	args = append(args, endpoint)
	out, err := run(ctx, "", nil, "tea", args...)
	if err != nil {
		return nil, err
	}
	if err := apiError(out); err != nil {
		return nil, i18n.Errorf("tea api %s: %w", endpoint, err)
	}
	return out, nil
}

// ---------- payloads ----------

type gtUser struct {
	Login    string `json:"login"`
	UserName string `json:"username"`
}

func (u *gtUser) name() string {
	switch {
	case u == nil:
		return ""
	case u.Login != "":
		return u.Login
	}
	return u.UserName
}

type gtLabel struct {
	Name string `json:"name"`
}

type gtRepo struct {
	FullName string `json:"full_name"`
}

// gtIssue is what /repos/issues/search returns, for pull requests as much as
// for issues: no branches, no diff stats, no reviews and no CI.
type gtIssue struct {
	Number      int       `json:"number"`
	Title       string    `json:"title"`
	Body        string    `json:"body"`
	HTMLURL     string    `json:"html_url"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
	Comments    int       `json:"comments"`
	User        *gtUser   `json:"user"`
	Labels      []gtLabel `json:"labels"`
	Assignees   []*gtUser `json:"assignees"`
	Repository  *gtRepo   `json:"repository"`
	PullRequest *struct {
		Draft bool `json:"draft"`
	} `json:"pull_request"`
}

type gtBranch struct {
	Ref  string  `json:"ref"`
	Sha  string  `json:"sha"`
	Repo *gtRepo `json:"repo"`
}

type gtPull struct {
	Draft        bool      `json:"draft"`
	Mergeable    bool      `json:"mergeable"`
	Additions    int       `json:"additions"`
	Deletions    int       `json:"deletions"`
	ChangedFiles int       `json:"changed_files"`
	Head         *gtBranch `json:"head"`
	Base         *gtBranch `json:"base"`
}

// gtRepoPull is the list representation returned by
// /repos/{owner}/{repo}/pulls. The detail-only fields are filled later by the
// same enrichment pass used for relation searches.
type gtRepoPull struct {
	Number    int       `json:"number"`
	Title     string    `json:"title"`
	Body      string    `json:"body"`
	HTMLURL   string    `json:"html_url"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
	Comments  int       `json:"comments"`
	User      *gtUser   `json:"user"`
	Labels    []gtLabel `json:"labels"`
	Assignees []*gtUser `json:"assignees"`
	Draft     bool      `json:"draft"`
}

type gtReview struct {
	ID          int64     `json:"id"`
	State       string    `json:"state"`
	Body        string    `json:"body"`
	User        *gtUser   `json:"user"`
	Dismissed   bool      `json:"dismissed"`
	Comments    int       `json:"comments_count"`
	SubmittedAt time.Time `json:"submitted_at"`
}

type gtReviewComment struct {
	Body      string    `json:"body"`
	Path      string    `json:"path"`
	Position  int       `json:"position"`
	OrigPos   int       `json:"original_position"`
	CreatedAt time.Time `json:"created_at"`
	User      *gtUser   `json:"user"`
}

type gtComment struct {
	Body      string    `json:"body"`
	CreatedAt time.Time `json:"created_at"`
	User      *gtUser   `json:"user"`
}

type gtStatus struct {
	State      string `json:"state"`
	TotalCount int    `json:"total_count"`
	Statuses   []struct {
		Status      string `json:"status"`
		Context     string `json:"context"`
		Description string `json:"description"`
		TargetURL   string `json:"target_url"`
	} `json:"statuses"`
}

// ---------- list ----------

// giteaSearches are the six queries behind the PR and issue tabs.
var giteaSearches = []struct {
	kind  Kind
	rel   Relation
	query string
}{
	{KindPR, ReviewRequested, "type=pulls&review_requested=true"},
	{KindPR, Authored, "type=pulls&created=true"},
	{KindPR, Assigned, "type=pulls&assigned=true"},
	{KindIssue, Assigned, "type=issues&assigned=true"},
	{KindIssue, Authored, "type=issues&created=true"},
	{KindIssue, Mentioned, "type=issues&mentioned=true"},
}

func (g *gitea) List(ctx context.Context) ([]Item, error) {
	me, err := g.currentUser(ctx)
	if err != nil {
		return nil, err
	}
	found := make([][]gtIssue, len(giteaSearches))
	errs := make([]error, len(giteaSearches))
	var wg sync.WaitGroup
	for i, s := range giteaSearches {
		wg.Add(1)
		go func() {
			defer wg.Done()
			// 50 matches the page size gh and glab use here; Gitea truncates
			// anything above max_response_items (50 by default) regardless.
			out, err := g.api(ctx, "", "/repos/issues/search?state=open&limit=50&"+s.query)
			if err != nil {
				errs[i] = err
				return
			}
			if err := json.Unmarshal(out, &found[i]); err != nil {
				errs[i] = i18n.Errorf("resposta inválida do tea: %w", err)
			}
		}()
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			return nil, err
		}
	}
	acc := newAccumulator()
	for i, s := range giteaSearches {
		for _, n := range found[i] {
			if n.Repository == nil {
				continue
			}
			acc.add(g.convert(n, s.kind), s.rel)
		}
	}
	for _, repo := range g.tracked {
		seen := map[int]bool{}
		for page := 1; ; page++ {
			out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/pulls?state=open&limit=50&page=%d", repo, page))
			if err != nil {
				return nil, err
			}
			var pulls []gtRepoPull
			if err := json.Unmarshal(out, &pulls); err != nil {
				return nil, i18n.Errorf("resposta inválida do tea para %s: %w", repo, err)
			}
			newPageItem := false
			for _, pull := range pulls {
				if !seen[pull.Number] {
					newPageItem = true
					seen[pull.Number] = true
				}
				draft := pull.Draft
				acc.add(g.convert(gtIssue{
					Number: pull.Number, Title: pull.Title, Body: pull.Body, HTMLURL: pull.HTMLURL,
					CreatedAt: pull.CreatedAt, UpdatedAt: pull.UpdatedAt, Comments: pull.Comments,
					User: pull.User, Labels: pull.Labels, Assignees: pull.Assignees,
					Repository: &gtRepo{FullName: repo}, PullRequest: &struct {
						Draft bool `json:"draft"`
					}{Draft: draft},
				}, KindPR), 0)
			}
			if len(pulls) < 50 || !newPageItem {
				break
			}
		}
	}
	items := acc.list()
	g.enrich(ctx, items, me)
	return items, nil
}

// currentUser doubles as the authentication check: when tea has no usable
// token the search endpoints still answer 200 with whatever is public, which
// would look like an empty list instead of an error. /user does not.
func (g *gitea) currentUser(ctx context.Context) (string, error) {
	out, err := g.api(ctx, "", "/user")
	if err != nil {
		var apiErr *giteaAPIError
		if errors.As(err, &apiErr) {
			return "", i18n.Errorf("tea não autenticado em %s (%s). Rode: tea login add --url %s",
				g.in.Host, apiErr.Message, g.serverURL())
		}
		return "", err
	}
	var u gtUser
	if err := json.Unmarshal(out, &u); err != nil {
		return "", i18n.Errorf("resposta inválida do tea: %w", err)
	}
	return u.name(), nil
}

func (g *gitea) convert(n gtIssue, kind Kind) Item {
	it := Item{
		Kind:      kind,
		Instance:  g.in.Name,
		Provider:  config.Gitea,
		Host:      g.in.Host,
		Repo:      n.Repository.FullName,
		RepoURL:   g.serverURL() + "/" + n.Repository.FullName,
		Number:    n.Number,
		Title:     n.Title,
		URL:       n.HTMLURL,
		Author:    n.User.name(),
		CreatedAt: n.CreatedAt,
		UpdatedAt: n.UpdatedAt,
		Comments:  n.Comments,
	}
	for _, l := range n.Labels {
		it.Labels = append(it.Labels, l.Name)
	}
	for _, a := range n.Assignees {
		it.Assignees = append(it.Assignees, a.name())
	}
	if n.PullRequest != nil {
		it.Draft = n.PullRequest.Draft
	}
	if kind == KindPR {
		// Gitea aggregates no review decision; enrich replaces this with what
		// the reviews actually say.
		it.Review = ReviewRequired
	}
	return it
}

// enrich fills in what the search leaves out. There is no bulk endpoint for any
// of it, so this costs up to three requests per pull request. Failures are
// tolerated on purpose: a row without CI beats losing the whole list.
func (g *gitea) enrich(ctx context.Context, items []Item, me string) {
	sem := make(chan struct{}, giteaWorkers)
	var wg sync.WaitGroup
	for i := range items {
		if items[i].IsIssue() {
			continue
		}
		wg.Add(1)
		go func(pr *Item) {
			defer wg.Done()
			select {
			case sem <- struct{}{}:
			case <-ctx.Done():
				return
			}
			defer func() { <-sem }()
			g.fillPull(ctx, pr, me)
		}(&items[i])
	}
	wg.Wait()
}

func (g *gitea) fillPull(ctx context.Context, pr *Item, me string) {
	out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/pulls/%d", pr.Repo, pr.Number))
	if err != nil {
		return
	}
	var d gtPull
	if json.Unmarshal(out, &d) != nil {
		return
	}
	applyPull(pr, d)
	if d.Head != nil && d.Head.Sha != "" {
		if out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/commits/%s/status", pr.Repo, d.Head.Sha)); err == nil {
			var st gtStatus
			if json.Unmarshal(out, &st) == nil {
				g.applyStatus(pr, st)
			}
		}
	}
	if out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/pulls/%d/reviews", pr.Repo, pr.Number)); err == nil {
		var rs []gtReview
		if json.Unmarshal(out, &rs) == nil {
			applyReviews(pr, rs, me)
		}
	}
}

// applyPull copies the fields only the pull request endpoint carries.
func applyPull(pr *Item, d gtPull) {
	pr.Draft = d.Draft
	pr.Additions, pr.Deletions, pr.Files = d.Additions, d.Deletions, d.ChangedFiles
	if d.Head != nil {
		pr.SourceBranch = d.Head.Ref
	}
	if d.Base != nil {
		pr.TargetBranch = d.Base.Ref
	}
	if d.Head != nil && d.Head.Repo != nil && d.Base != nil && d.Base.Repo != nil {
		pr.FromFork = !strings.EqualFold(d.Head.Repo.FullName, d.Base.Repo.FullName)
	}
	// Gitea also reports drafts, and pull requests whose conflict check is
	// still running, as not mergeable. Drafts are filtered out here; a check in
	// flight resolves itself on the next refresh.
	pr.Conflicts = !d.Mergeable && !d.Draft
}

func (g *gitea) applyStatus(pr *Item, st gtStatus) {
	if st.TotalCount == 0 {
		return
	}
	pr.CI = gtState(st.State)
	for _, s := range st.Statuses {
		name := s.Context
		if name == "" {
			name = s.Description
		}
		pr.Checks = append(pr.Checks, Check{Name: name, State: gtState(s.Status), URL: g.absolute(s.TargetURL)})
	}
}

// absolute completes the relative target_url that Gitea Actions write.
func (g *gitea) absolute(u string) string {
	if strings.HasPrefix(u, "/") {
		return g.serverURL() + u
	}
	return u
}

func gtState(s string) CIState {
	switch strings.ToLower(s) {
	case "success", "warning":
		return CISuccess
	case "failure", "error":
		return CIFailure
	case "skipped":
		return CICanceled
	case "pending":
		return CIPending
	}
	return CINone
}

// applyReviews derives the review state Gitea does not aggregate. Reviews come
// back oldest first, so the last one of each user is the one that counts.
func applyReviews(pr *Item, rs []gtReview, me string) {
	latest := map[string]string{}
	var order []string
	for _, r := range rs {
		name := r.User.name()
		if name == "" || r.Dismissed {
			continue
		}
		state := strings.ToUpper(r.State)
		if state != "APPROVED" && state != "REQUEST_CHANGES" {
			continue // plain comments and unsubmitted drafts decide nothing
		}
		if _, seen := latest[name]; !seen {
			order = append(order, name)
		}
		latest[name] = state
	}
	changes := false
	pr.ApprovedBy, pr.ApprovedByMe = nil, false
	for _, name := range order {
		if latest[name] != "APPROVED" {
			changes = true
			continue
		}
		pr.ApprovedBy = append(pr.ApprovedBy, name)
		if strings.EqualFold(name, me) {
			pr.ApprovedByMe = true
		}
	}
	switch {
	case changes:
		pr.Review = ReviewChangesRequested
	case len(pr.ApprovedBy) > 0:
		pr.Review = ReviewApproved
	default:
		pr.Review = ReviewRequired
	}
}

// ---------- thread ----------

func (g *gitea) Thread(ctx context.Context, it *Item) (*Thread, error) {
	// Pull requests are issues in Gitea, so the description and the plain
	// comments come from the same endpoints for both kinds.
	out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/issues/%d", it.Repo, it.Number))
	if err != nil {
		return nil, err
	}
	var head gtIssue
	if err := json.Unmarshal(out, &head); err != nil {
		return nil, i18n.Errorf("resposta inválida do tea: %w", err)
	}
	t := &Thread{Body: head.Body}
	out, err = g.api(ctx, "", fmt.Sprintf("/repos/%s/issues/%d/comments", it.Repo, it.Number))
	if err != nil {
		return nil, err
	}
	var cs []gtComment
	if err := json.Unmarshal(out, &cs); err != nil {
		return nil, i18n.Errorf("resposta inválida do tea: %w", err)
	}
	for _, c := range cs {
		t.Comments = append(t.Comments, Comment{Author: c.User.name(), Body: c.Body, CreatedAt: c.CreatedAt})
	}
	if !it.IsIssue() {
		if err := g.appendReviews(ctx, it, t); err != nil {
			return nil, err
		}
	}
	sortComments(t.Comments)
	return t, nil
}

// appendReviews adds the review bodies and the inline code comments, which
// Gitea keeps out of the issue comment list.
func (g *gitea) appendReviews(ctx context.Context, it *Item, t *Thread) error {
	out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/pulls/%d/reviews", it.Repo, it.Number))
	if err != nil {
		return err
	}
	var rs []gtReview
	if err := json.Unmarshal(out, &rs); err != nil {
		return i18n.Errorf("resposta inválida do tea: %w", err)
	}
	for _, r := range rs {
		if strings.EqualFold(r.State, "PENDING") {
			continue // a draft review, visible only to its author
		}
		state := gtReviewState(r)
		if r.Body != "" || state == ReviewApproved || state == ReviewChangesRequested {
			t.Comments = append(t.Comments, Comment{
				Author:    r.User.name(),
				Body:      r.Body,
				CreatedAt: r.SubmittedAt,
				Review:    state,
			})
		}
		if r.Comments == 0 {
			continue
		}
		out, err := g.api(ctx, "", fmt.Sprintf("/repos/%s/pulls/%d/reviews/%d/comments", it.Repo, it.Number, r.ID))
		if err != nil {
			return err
		}
		var rcs []gtReviewComment
		if err := json.Unmarshal(out, &rcs); err != nil {
			return i18n.Errorf("resposta inválida do tea: %w", err)
		}
		for _, rc := range rcs {
			line := rc.Position
			if line == 0 {
				line = rc.OrigPos
			}
			t.Comments = append(t.Comments, Comment{
				Author:    rc.User.name(),
				Body:      rc.Body,
				CreatedAt: rc.CreatedAt,
				Path:      rc.Path,
				Line:      line,
			})
		}
	}
	return nil
}

// gtReviewState maps a Gitea review to the vocabulary the UI renders.
func gtReviewState(r gtReview) string {
	if r.Dismissed {
		return "dismissed"
	}
	switch strings.ToUpper(r.State) {
	case "APPROVED":
		return ReviewApproved
	case "REQUEST_CHANGES":
		return ReviewChangesRequested
	case "COMMENT":
		return "commented"
	}
	return ""
}

// ---------- actions ----------

func (g *gitea) AddComment(ctx context.Context, it *Item, body string) error {
	_, err := g.api(ctx, "POST", fmt.Sprintf("/repos/%s/issues/%d/comments", it.Repo, it.Number), "-f", "body="+body)
	return err
}

func (g *gitea) Approve(ctx context.Context, pr *Item) error {
	login, err := g.resolve(ctx)
	if err != nil {
		return err
	}
	_, err = run(ctx, "", nil, "tea", "pulls", "approve", strconv.Itoa(pr.Number), "--repo", pr.Repo, "--login", login)
	return err
}

// Merge goes through the API because "tea pulls merge" only exposes --style,
// while the instance settings also offer deleting the source branch and
// merging as soon as the checks pass.
func (g *gitea) Merge(ctx context.Context, pr *Item, opts MergeOptions) error {
	method := mergeMethod(opts.Method)
	fields := []string{
		// Gitea binds the field as "Do"; 1.23 and newer also accept "do".
		"-f", "Do=" + method,
		"-f", "do=" + method,
		"-F", "delete_branch_after_merge=" + strconv.FormatBool(opts.DeleteBranch),
		"-F", "merge_when_checks_succeed=" + strconv.FormatBool(opts.Auto),
	}
	_, err := g.api(ctx, "POST", fmt.Sprintf("/repos/%s/pulls/%d/merge", pr.Repo, pr.Number), fields...)
	return err
}

// Close closes the pull request and leaves the source branch alone.
func (g *gitea) Close(ctx context.Context, pr *Item) error {
	login, err := g.resolve(ctx)
	if err != nil {
		return err
	}
	_, err = run(ctx, "", nil, "tea", "pulls", "close", strconv.Itoa(pr.Number), "--repo", pr.Repo, "--login", login)
	return err
}

// Checkout runs inside the clone so tea resolves the repository from its
// remote, the same way gh and glab do.
func (g *gitea) Checkout(ctx context.Context, pr *Item, dir string) error {
	login, err := g.resolve(ctx)
	if err != nil {
		return err
	}
	_, err = run(ctx, dir, nil, "tea", "pulls", "checkout", strconv.Itoa(pr.Number), "--branch", "--login", login)
	return err
}

// HeadRef is the ref Gitea keeps on the base repository, forks included.
func (g *gitea) HeadRef(pr *Item) string { return fmt.Sprintf("refs/pull/%d/head", pr.Number) }

func (g *gitea) AuthStatus(ctx context.Context) error {
	_, err := g.currentUser(ctx)
	return err
}

// GiteaInstances returns one instance per tea login. Gitea is self-hosted far
// more often than not, so there is no SaaS host worth probing blindly: what the
// user already logged tea into is the useful seed.
func GiteaInstances(ctx context.Context) []config.Instance {
	if !ToolAvailable("tea") {
		return nil
	}
	out, err := run(ctx, "", nil, "tea", "logins", "list", "--output", "json")
	if err != nil {
		return nil
	}
	var logins []teaLogin
	if json.Unmarshal(out, &logins) != nil {
		return nil
	}
	var instances []config.Instance
	for _, l := range logins {
		host := config.NormalizeHost(l.URL)
		if host == "" {
			continue
		}
		instances = append(instances, config.Instance{Name: l.Name, Provider: config.Gitea, Host: host})
	}
	return instances
}
