package provider

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

type draftInvocation struct {
	Args       []string
	GitHubHost string
	GitLabHost string
}

// Reuse the test executable as a CLI stub on every OS. These tests never run
// an authenticated CLI or make network requests.
func TestMain(m *testing.M) {
	if os.Getenv("PR_TRACKER_DRAFT_CLI_HELPER") == "1" {
		data, _ := json.Marshal(draftInvocation{os.Args[1:], os.Getenv("GH_HOST"), os.Getenv("GITLAB_HOST")})
		f, err := os.OpenFile(os.Getenv("PR_TRACKER_DRAFT_CLI_LOG"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
		if err != nil {
			panic(err)
		}
		fmt.Fprintln(f, string(data))
		f.Close()
		if os.Getenv("PR_TRACKER_DRAFT_CLI_FAIL") == "1" {
			fmt.Fprintln(os.Stderr, "permission denied")
			os.Exit(1)
		}
		method := ""
		for i, arg := range os.Args {
			if arg == "--method" && i+1 < len(os.Args) {
				method = os.Args[i+1]
			}
		}
		response := "{}"
		if method == "GET" {
			response = os.Getenv("PR_TRACKER_DRAFT_CLI_GET")
		}
		if method == "PATCH" {
			response = os.Getenv("PR_TRACKER_DRAFT_CLI_PATCH")
		}
		fmt.Println(response)
		os.Exit(0)
	}
	os.Exit(m.Run())
}

func draftCLI(t *testing.T) (string, string) {
	t.Helper()
	path, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	log := filepath.Join(t.TempDir(), "calls.jsonl")
	t.Setenv("PR_TRACKER_DRAFT_CLI_HELPER", "1")
	t.Setenv("PR_TRACKER_DRAFT_CLI_LOG", log)
	return path, log
}

func draftCalls(t *testing.T, log string) []draftInvocation {
	t.Helper()
	data, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	var calls []draftInvocation
	for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		var call draftInvocation
		if err := json.Unmarshal([]byte(line), &call); err != nil {
			t.Fatal(err)
		}
		calls = append(calls, call)
	}
	return calls
}

func TestGitHubAndGitLabDraftCommands(t *testing.T) {
	for _, platform := range []config.Provider{config.GitHub, config.GitLab} {
		for _, draft := range []bool{false, true} {
			t.Run(string(platform)+fmt.Sprint(draft), func(t *testing.T) {
				path, log := draftCLI(t)
				client := New(config.Instance{Name: "work", Provider: platform, Host: "git.company.com"})
				ctx := toolchain.WithPaths(t.Context(), map[string]string{client.Tool(): path})
				item := Item{Repo: "team/sub/service", Number: 42}
				if err := client.SetDraft(ctx, &item, draft); err != nil {
					t.Fatal(err)
				}
				calls := draftCalls(t, log)
				if len(calls) != 1 {
					t.Fatalf("calls = %+v", calls)
				}
				var want []string
				if platform == config.GitHub {
					want = []string{"pr", "ready", "42", "-R", "git.company.com/team/sub/service"}
					if draft {
						want = append(want, "--undo")
					}
					if calls[0].GitHubHost != "git.company.com" {
						t.Fatal("wrong GitHub host")
					}
				} else {
					flag := "--ready"
					if draft {
						flag = "--draft"
					}
					want = []string{"mr", "update", "42", "-R", "https://git.company.com/team/sub/service", flag, "--yes"}
					if calls[0].GitLabHost != "git.company.com" {
						t.Fatal("wrong GitLab host")
					}
				}
				if !reflect.DeepEqual(calls[0].Args, want) {
					t.Fatalf("calls = %+v, want %v", calls, want)
				}
				t.Setenv("PR_TRACKER_DRAFT_CLI_FAIL", "1")
				if err := client.SetDraft(ctx, &item, draft); err == nil {
					t.Fatal("CLI failure ignored")
				}
			})
		}
	}
}

func TestGiteaDraftTransitions(t *testing.T) {
	for _, tc := range []struct {
		name, before, after, title string
		draft, fail                bool
		calls                      int
	}{
		{"draft", `{"title":"New title","draft":false,"content_version":7}`, `{"title":"WIP: New title","draft":true}`, "WIP: New title", true, false, 2},
		{"ready", `{"title":"[wip] New title","draft":true}`, `{"title":"New title","draft":false}`, "New title", false, false, 2},
		{"already ready", `{"title":"New title","draft":false}`, "", "", false, false, 1},
		{"custom prefix", `{"title":"Custom: New title","draft":true}`, "", "", false, true, 1},
		{"missing state", `{"title":"New title"}`, "", "", true, true, 1},
		{"unrecognized WIP marker", `{"title":"New title","draft":false}`, `{"title":"WIP: New title","draft":false}`, "WIP: New title", true, true, 2},
	} {
		t.Run(tc.name, func(t *testing.T) {
			path, log := draftCLI(t)
			t.Setenv("PR_TRACKER_DRAFT_CLI_GET", tc.before)
			t.Setenv("PR_TRACKER_DRAFT_CLI_PATCH", tc.after)
			client := giteaClient()
			client.login = "corp-account"
			ctx := toolchain.WithPaths(t.Context(), map[string]string{"tea": path})
			item := Item{Repo: "team/service", Number: 9, Title: "Stale cached title"}
			err := client.SetDraft(ctx, &item, tc.draft)
			if (err != nil) != tc.fail {
				t.Fatalf("error = %v", err)
			}
			calls := draftCalls(t, log)
			if len(calls) != tc.calls {
				t.Fatalf("calls = %+v", calls)
			}
			wantGET := []string{"api", "--login", "corp-account", "--method", "GET", "/repos/team/service/pulls/9"}
			if !reflect.DeepEqual(calls[0].Args, wantGET) {
				t.Fatalf("GET = %+v", calls[0])
			}
			if tc.calls == 2 {
				want := []string{"api", "--login", "corp-account", "--method", "PATCH", "-f", "title=" + tc.title}
				if strings.Contains(tc.before, "content_version") {
					want = append(want, "-F", "content_version=7")
				}
				want = append(want, "/repos/team/service/pulls/9")
				if !reflect.DeepEqual(calls[1].Args, want) {
					t.Fatalf("PATCH = %+v, want %v", calls[1], want)
				}
			}
		})
	}
}
