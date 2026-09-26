// Package toolchain resolves CLIs for terminal and desktop sessions.
package toolchain

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

type pathsKey struct{}

// WithPaths supplies per-application overrides without changing process PATH.
func WithPaths(ctx context.Context, paths map[string]string) context.Context {
	return context.WithValue(ctx, pathsKey{}, paths)
}

func Lookup(ctx context.Context, name string) (string, error) {
	if paths, ok := ctx.Value(pathsKey{}).(map[string]string); ok && paths[name] != "" {
		return exec.LookPath(paths[name])
	}
	path, err := exec.LookPath(name)
	if err == nil || runtime.GOOS != "darwin" {
		return path, err
	}
	// Finder does not necessarily inherit Homebrew's shell PATH.
	for _, dir := range []string{"/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"} {
		if path, e := exec.LookPath(filepath.Join(dir, name)); e == nil {
			return path, nil
		}
	}
	return "", err
}

// Environment lets CLIs find their own child tools in the same desktop session.
// It affects only the child process, never the application-wide environment.
func Environment(ctx context.Context) []string {
	paths, _ := ctx.Value(pathsKey{}).(map[string]string)
	var dirs []string
	names := make([]string, 0, len(paths))
	for name := range paths {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		if filepath.IsAbs(paths[name]) {
			dirs = append(dirs, filepath.Dir(paths[name]))
		}
	}
	if runtime.GOOS == "darwin" {
		dirs = append(dirs, "/opt/homebrew/bin", "/usr/local/bin")
	}
	dirs = append(dirs, os.Getenv("PATH"))
	env := os.Environ()
	for i, value := range env {
		if strings.HasPrefix(strings.ToUpper(value), "PATH=") {
			env[i] = "PATH=" + strings.Join(dirs, string(os.PathListSeparator))
			return env
		}
	}
	return append(env, "PATH="+strings.Join(dirs, string(os.PathListSeparator)))
}
