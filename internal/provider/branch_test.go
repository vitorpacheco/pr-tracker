package provider

import (
	"reflect"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

func TestUpdateBranchCommands(t *testing.T) {
	for _, platform := range []config.Provider{config.GitHub, config.GitLab, config.Gitea} {
		for _, fail := range []bool{false, true} {
			t.Run(string(platform)+map[bool]string{false: "/success", true: "/failure"}[fail], func(t *testing.T) {
				path, log := draftCLI(t)
				if fail {
					t.Setenv("PR_TRACKER_DRAFT_CLI_FAIL", "1")
				}
				instance := config.Instance{Name: "work", Provider: platform, Host: "git.company.com"}
				client := New(instance)
				if platform == config.Gitea {
					client = &gitea{in: instance, login: "work"}
				}
				ctx := toolchain.WithPaths(t.Context(), map[string]string{client.Tool(): path})
				err := client.UpdateBranch(ctx, &Item{Repo: "team/service", Number: 42})
				if (err != nil) != fail {
					t.Fatalf("error = %v, failure %v", err, fail)
				}
				calls := draftCalls(t, log)
				if len(calls) != 1 {
					t.Fatalf("calls = %+v", calls)
				}
				var want []string
				switch platform {
				case config.GitHub:
					want = []string{"pr", "update-branch", "42", "-R", "git.company.com/team/service"}
					if calls[0].GitHubHost != instance.Host {
						t.Fatal("wrong GitHub host")
					}
				case config.GitLab:
					want = []string{"mr", "rebase", "42", "-R", "https://git.company.com/team/service"}
					if calls[0].GitLabHost != instance.Host {
						t.Fatal("wrong GitLab host")
					}
				case config.Gitea:
					want = []string{"api", "--login", "work", "--method", "POST", "/repos/team/service/pulls/42/update?style=merge"}
				}
				if !reflect.DeepEqual(calls[0].Args, want) {
					t.Fatalf("args = %v, want %v", calls[0].Args, want)
				}
			})
		}
	}
}

func TestGitHubBranchUpdateState(t *testing.T) {
	g := &github{in: config.Instance{Provider: config.GitHub}}
	for _, tc := range []struct {
		status      string
		can, behind bool
	}{
		{"BEHIND", false, true}, {"CLEAN", true, true}, {"CLEAN", false, false}, {"UNKNOWN", false, false},
	} {
		item := g.convert(ghPR{MergeStateStatus: tc.status, ViewerCanUpdateBranch: tc.can}, "me")
		if item.BehindBase != tc.behind || item.CanUpdateBranch != tc.can {
			t.Fatalf("state = %+v, want %+v", item, tc)
		}
	}
}
