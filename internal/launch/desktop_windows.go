package launch

import (
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
)

func configureDesktopProcess(cmd *exec.Cmd) {
	name := strings.ToLower(filepath.Base(cmd.Path))
	if name == "powershell.exe" || name == "pwsh.exe" {
		cmd.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x00000010}
	}
}
