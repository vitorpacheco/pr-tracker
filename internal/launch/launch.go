// Package launch opens shells and tools in a directory, using herdr or tmux
// when available and falling back to suspending the TUI.
package launch

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"

	"github.com/vitorpacheco/pr-tracker/internal/i18n"
)

// Mode is where a command opens.
type Mode string

const (
	Herdr  Mode = "herdr"
	Tmux   Mode = "tmux"
	Inline Mode = "inline"
)

// Resolve turns the configured terminal setting into a concrete mode.
func Resolve(setting string) Mode {
	switch setting {
	case "herdr":
		if _, err := exec.LookPath("herdr"); err == nil {
			return Herdr
		}
	case "tmux":
		if os.Getenv("TMUX") != "" {
			return Tmux
		}
	case "inline":
		return Inline
	default: // auto
		if os.Getenv("HERDR_ENV") == "1" {
			if _, err := exec.LookPath("herdr"); err == nil {
				return Herdr
			}
		}
		if os.Getenv("TMUX") != "" {
			return Tmux
		}
	}
	return Inline
}

// Open runs argv (or an interactive shell when argv is empty) in dir.
// For Inline mode it returns the command for the caller to run while the TUI
// is suspended; otherwise the command is started in a new tab/window.
func Open(mode Mode, dir, label string, argv []string) (*exec.Cmd, error) {
	switch mode {
	case Herdr:
		return nil, openHerdr(dir, label, argv)
	case Tmux:
		args := []string{"new-window", "-c", dir, "-n", label}
		if len(argv) > 0 {
			args = append(args, Quote(argv))
		}
		if out, err := exec.Command("tmux", args...).CombinedOutput(); err != nil {
			return nil, fmt.Errorf("tmux: %s", strings.TrimSpace(string(out)))
		}
		return nil, nil
	}
	if len(argv) == 0 {
		argv = Shell()
	}
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Dir = dir
	return cmd, nil
}

func openHerdr(dir, label string, argv []string) error {
	out, err := exec.Command("herdr", "tab", "create", "--cwd", dir, "--label", label, "--focus").Output()
	if err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) {
			return fmt.Errorf("herdr: %s", strings.TrimSpace(string(ee.Stderr)))
		}
		return fmt.Errorf("herdr: %w", err)
	}
	if len(argv) == 0 {
		return nil
	}
	var resp struct {
		Result struct {
			RootPane struct {
				PaneID string `json:"pane_id"`
			} `json:"root_pane"`
		} `json:"result"`
	}
	if err := json.Unmarshal(bytes.TrimSpace(out), &resp); err != nil || resp.Result.RootPane.PaneID == "" {
		return i18n.Errorf("herdr: resposta inesperada de tab create: %s", strings.TrimSpace(string(out)))
	}
	if out, err := exec.Command("herdr", "pane", "run", resp.Result.RootPane.PaneID, Quote(argv)).CombinedOutput(); err != nil {
		return fmt.Errorf("herdr: %s", strings.TrimSpace(string(out)))
	}
	return nil
}

// Shell returns the user's interactive shell.
func Shell() []string {
	if runtime.GOOS == "windows" {
		if c := os.Getenv("COMSPEC"); c != "" {
			return []string{c}
		}
		return []string{"cmd.exe"}
	}
	if s := os.Getenv("SHELL"); s != "" {
		return []string{s}
	}
	return []string{"/bin/sh"}
}

// Quote joins argv into a POSIX shell command line.
func Quote(argv []string) string {
	parts := make([]string, len(argv))
	for i, a := range argv {
		if a != "" && strings.IndexFunc(a, func(r rune) bool {
			return !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || strings.ContainsRune("-_./=:@%+,", r))
		}) < 0 {
			parts[i] = a
			continue
		}
		parts[i] = "'" + strings.ReplaceAll(a, "'", `'\''`) + "'"
	}
	return strings.Join(parts, " ")
}

// Browser opens a URL with the platform opener.
func Browser(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", url)
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	cmd.Stdout, cmd.Stderr = nil, nil
	if err := cmd.Start(); err != nil {
		return err
	}
	go cmd.Wait() //nolint:errcheck
	return nil
}
