package launch

import (
	"context"
	"fmt"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"

	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

// Desktop opens a standalone terminal without suspending or using the GUI's
// stdio. Commands are passed as argv; only macOS Terminal requires shell text.
func Desktop(ctx context.Context, terminal, dir string, argv []string) error {
	name, args, err := desktopCommand(runtime.GOOS, terminal, dir, argv, func(name string) (string, error) { return toolchain.Lookup(ctx, name) })
	if err != nil {
		return err
	}
	cmd := exec.Command(name, args...)
	cmd.Dir = dir
	cmd.Env = toolchain.Environment(ctx)
	configureDesktopProcess(cmd)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("abrindo terminal: %w", err)
	}
	go func() { _ = cmd.Wait() }()
	return nil
}

func desktopCommand(platform, terminal, dir string, argv []string, lookup func(string) (string, error)) (string, []string, error) {
	if terminal == "" {
		if platform == "darwin" {
			command := "cd -- " + Quote([]string{dir})
			if len(argv) > 0 {
				command += " && " + Quote(argv)
			}
			return "/usr/bin/osascript", []string{"-e", `tell application "Terminal" to do script ` + strconv.Quote(command), "-e", `tell application "Terminal" to activate`}, nil
		}
		candidates := []string{"kitty", "foot", "wezterm", "gnome-terminal", "konsole", "xterm"}
		if platform == "windows" {
			candidates = []string{"wt.exe", "pwsh.exe", "powershell.exe"}
		}
		for _, candidate := range candidates {
			if path, err := lookup(candidate); err == nil {
				terminal = path
				break
			}
		}
	}
	if terminal == "" {
		return "", nil, fmt.Errorf("nenhum terminal desktop encontrado; configure desktop_terminal")
	}
	path, err := lookup(terminal)
	if err != nil {
		return "", nil, err
	}
	base := strings.TrimSuffix(filepath.Base(terminal), ".exe")
	switch base {
	case "pwsh", "powershell":
		args := []string{"-NoExit", "-NoLogo"}
		if len(argv) > 0 {
			quoted := make([]string, len(argv))
			for i, arg := range argv {
				quoted[i] = "'" + strings.ReplaceAll(arg, "'", "''") + "'"
			}
			args = append(args, "-Command", "& "+strings.Join(quoted, " "))
		}
		return path, args, nil
	case "wt":
		args := []string{"-d", dir}
		return path, append(args, argv...), nil
	case "wezterm":
		args := []string{"start", "--cwd", dir}
		if len(argv) > 0 {
			args = append(args, "--")
			args = append(args, argv...)
		}
		return path, args, nil
	case "gnome-terminal":
		args := []string{"--working-directory=" + dir}
		if len(argv) > 0 {
			args = append(args, "--")
			args = append(args, argv...)
		}
		return path, args, nil
	default:
		if len(argv) == 0 {
			return path, nil, nil
		}
		return path, append([]string{"-e"}, argv...), nil
	}
}

func Folder(dir string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", dir)
	case "windows":
		cmd = exec.Command("explorer.exe", dir)
	default:
		cmd = exec.Command("xdg-open", dir)
	}
	if err := cmd.Start(); err != nil {
		return err
	}
	go func() { _ = cmd.Wait() }()
	return nil
}
