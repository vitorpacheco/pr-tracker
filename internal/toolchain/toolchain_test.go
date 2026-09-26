package toolchain

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestExplicitPathOverridesPATHAndDoesNotFallBack(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	paths := map[string]string{"gh": executable}
	ctx := WithPaths(context.Background(), paths)
	found, err := Lookup(ctx, "gh")
	if err != nil || found != executable {
		t.Fatalf("lookup = %q, %v", found, err)
	}
	paths["gh"] = "/nonexistent/pr-tracker-test-cli"
	if _, err := Lookup(ctx, "gh"); err == nil {
		t.Fatal("invalid explicit path silently fell back")
	}
}

func TestChildEnvironmentDoesNotMutateProcess(t *testing.T) {
	before := os.Getenv("PATH")
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	env := Environment(WithPaths(context.Background(), map[string]string{"git": exe}))
	found := false
	for _, entry := range env {
		if strings.HasPrefix(entry, "PATH=") {
			found = strings.HasPrefix(entry, "PATH="+filepath.Dir(exe)+string(os.PathListSeparator))
		}
	}
	if !found || os.Getenv("PATH") != before {
		t.Fatal("child PATH was not isolated")
	}
}
