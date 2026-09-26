package app

import (
	"context"
	"errors"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

type actionClient struct {
	provider.Client
	calls []string
	err   error
	opts  provider.MergeOptions
	body  string
}

func (c *actionClient) Approve(context.Context, *provider.Item) error {
	c.calls = append(c.calls, "approve")
	return c.err
}
func (c *actionClient) Merge(_ context.Context, _ *provider.Item, opts provider.MergeOptions) error {
	c.calls = append(c.calls, "merge")
	c.opts = opts
	return c.err
}
func (c *actionClient) Close(context.Context, *provider.Item) error {
	c.calls = append(c.calls, "close")
	return c.err
}
func (c *actionClient) AddComment(_ context.Context, _ *provider.Item, body string) error {
	c.calls = append(c.calls, "comment")
	c.body = body
	return c.err
}
func (c *actionClient) Checkout(_ context.Context, _ *provider.Item, dir string) error {
	c.calls = append(c.calls, "checkout:"+dir)
	return c.err
}
func (c *actionClient) Thread(context.Context, *provider.Item) (*provider.Thread, error) {
	c.calls = append(c.calls, "detail")
	return &provider.Thread{Body: "<details>raw markdown</details>"}, c.err
}

func actionFixture() (*Application, *actionClient, provider.Item) {
	cfg := config.Default()
	cfg.Instances = []config.Instance{{Name: "work", Provider: config.GitHub, Host: "github.com", MergeMethod: "squash", AutoMerge: true, DeleteBranch: true}}
	cfg.Repos = []config.Repo{{Instance: "work", Name: "org/repo", MergeMethod: "rebase"}}
	a := New(cfg, nil)
	client := &actionClient{}
	a.client = func(config.Instance, ...config.Repo) provider.Client { return client }
	a.resolveClone = func(context.Context, *config.Config, *provider.Item) (string, string, bool) {
		return "/clone", "upstream", true
	}
	a.worktreeExists = func(*config.Config, *provider.Item) (string, bool) { return "/worktree", true }
	a.createWorktree = func(context.Context, *config.Config, provider.Client, *provider.Item, string, string) (string, error) {
		return "/created", nil
	}
	a.removeWorktree = func(context.Context, *config.Config, *provider.Item, bool) error { return nil }
	return a, client, provider.Item{Instance: "work", Provider: config.GitHub, Host: "github.com", Repo: "org/repo", Number: 7}
}

func TestRemoteActionAndCleanupOutcomes(t *testing.T) {
	for _, action := range []Action{Approve, Merge, Close} {
		for _, scenario := range []string{"success", "remote failure", "dirty", "cleanup failure"} {
			t.Run(string(action)+"/"+scenario, func(t *testing.T) {
				a, c, item := actionFixture()
				cleanup := 0
				failure := errors.New("failure")
				if scenario == "remote failure" {
					c.err = failure
				}
				a.removeWorktree = func(_ context.Context, _ *config.Config, _ *provider.Item, force bool) error {
					cleanup++
					if force {
						t.Fatal("combined operation forced removal")
					}
					if len(c.calls) != 1 {
						t.Fatal("cleanup ran before remote action")
					}
					if scenario == "dirty" {
						return ErrDirtyWorktree
					}
					if scenario == "cleanup failure" {
						return failure
					}
					return nil
				}
				out, err := a.Execute(context.Background(), Command{Action: action, Item: item, RemoveAfter: true})
				if len(c.calls) != 1 || c.calls[0] != string(action) {
					t.Fatalf("calls = %v", c.calls)
				}
				if action == Merge && c.opts != (provider.MergeOptions{Method: "rebase", Auto: true, DeleteBranch: true}) {
					t.Fatalf("merge options = %+v", c.opts)
				}
				if scenario == "remote failure" {
					if cleanup != 0 || out.Refresh || out.Message != "" || !errors.Is(err, failure) {
						t.Fatalf("outcome = %+v, error = %v, cleanup = %d", out, err, cleanup)
					}
					return
				}
				if cleanup != 1 || !out.Refresh || out.Message == "" {
					t.Fatalf("lost remote success: %+v, %v", out, err)
				}
				switch scenario {
				case "success":
					if err != nil {
						t.Fatal(err)
					}
				case "dirty":
					if !errors.Is(err, ErrDirtyWorktree) {
						t.Fatal(err)
					}
				default:
					if !errors.Is(err, failure) {
						t.Fatal(err)
					}
				}
			})
		}
	}
}

func TestExecuteRejectsInvalidCommandsBeforeSideEffects(t *testing.T) {
	for _, scenario := range []string{"unknown", "issue approval", "empty comment", "force merge", "combined comment", "missing instance", "disabled instance", "changed host", "changed provider", "invalid item", "cancelled"} {
		t.Run(scenario, func(t *testing.T) {
			a, c, item := actionFixture()
			command := Command{Action: Approve, Item: item}
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			switch scenario {
			case "unknown":
				command.Action = "unknown"
			case "issue approval":
				command.Item.Kind = provider.KindIssue
			case "empty comment":
				command.Action = Comment
				command.Body = " \n "
			case "force merge":
				command.Action = Merge
				command.Force = true
			case "combined comment":
				command.Action = Comment
				command.Body = "hello"
				command.RemoveAfter = true
			case "missing instance":
				command.Item.Instance = "missing"
			case "disabled instance":
				a.cfg.Instances[0].Disabled = true
			case "changed host":
				command.Item.Host = "another.example"
			case "changed provider":
				command.Item.Provider = config.GitLab
			case "invalid item":
				command.Item.Number = 0
			case "cancelled":
				cancel()
			}
			if _, err := a.Execute(ctx, command); err == nil {
				t.Fatal("expected rejection")
			}
			if len(c.calls) != 0 {
				t.Fatalf("provider called: %v", c.calls)
			}
		})
	}
}

func TestCommentAndDetailForIssue(t *testing.T) {
	a, c, item := actionFixture()
	item.Kind = provider.KindIssue
	out, err := a.Execute(context.Background(), Command{Action: Comment, Item: item, Body: "  **hello**\n "})
	if err != nil || !out.Refresh || !out.ReloadThread || c.body != "**hello**" {
		t.Fatalf("comment = %+v, %v, %q", out, err, c.body)
	}
	detail, err := a.Detail(context.Background(), item)
	if err != nil || detail.Body != "<details>raw markdown</details>" {
		t.Fatalf("detail = %+v, %v", detail, err)
	}
}

func TestWorktreePreparationAndCheckout(t *testing.T) {
	for _, action := range []Action{UpdateWorktree, PrepareTerminal, Checkout} {
		t.Run(string(action), func(t *testing.T) {
			a, c, item := actionFixture()
			created := false
			a.createWorktree = func(_ context.Context, _ *config.Config, _ provider.Client, _ *provider.Item, clone, remote string) (string, error) {
				created = true
				if clone != "/clone" || remote != "upstream" {
					t.Fatalf("clone = %s, remote = %s", clone, remote)
				}
				return "/created", nil
			}
			out, err := a.Execute(context.Background(), Command{Action: action, Item: item})
			if err != nil {
				t.Fatal(err)
			}
			switch action {
			case UpdateWorktree:
				if !created || out.Directory != "/created" {
					t.Fatalf("worktree not updated: %+v", out)
				}
			case PrepareTerminal:
				if created || out.Directory != "/worktree" {
					t.Fatalf("existing worktree not reused: %+v", out)
				}
			case Checkout:
				if len(c.calls) != 1 || c.calls[0] != "checkout:/clone" {
					t.Fatalf("calls = %v", c.calls)
				}
			}
		})
	}
}

func TestMissingCloneAndForcedRemoval(t *testing.T) {
	a, _, item := actionFixture()
	a.worktreeExists = func(*config.Config, *provider.Item) (string, bool) { return "", false }
	a.resolveClone = func(context.Context, *config.Config, *provider.Item) (string, string, bool) { return "", "", false }
	for _, action := range []Action{Checkout, UpdateWorktree, PrepareTerminal, PrepareDiff} {
		if _, err := a.Execute(context.Background(), Command{Action: action, Item: item}); !errors.Is(err, ErrCloneRequired) {
			t.Fatalf("%s: %v", action, err)
		}
	}
	a.removeWorktree = func(_ context.Context, _ *config.Config, _ *provider.Item, force bool) error {
		if !force {
			return ErrDirtyWorktree
		}
		return nil
	}
	if _, err := a.Execute(context.Background(), Command{Action: RemoveWorktree, Item: item}); !errors.Is(err, ErrDirtyWorktree) {
		t.Fatal(err)
	}
	out, err := a.Execute(context.Background(), Command{Action: RemoveWorktree, Item: item, Force: true})
	if err != nil || out.Refresh || out.Message == "" {
		t.Fatalf("forced removal = %+v, %v", out, err)
	}
}
