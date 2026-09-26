//go:build !windows

package launch

import "os/exec"

func configureDesktopProcess(*exec.Cmd) {}
