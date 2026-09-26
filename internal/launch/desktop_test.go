package launch

import (
	"errors"
	"reflect"
	"strings"
	"testing"
)

func TestDesktopTerminalArguments(t *testing.T) {
	lookup := func(name string) (string, error) { return name, nil }
	for _, tc := range []struct {
		name string
		want []string
	}{
		{"kitty", []string{"-e", "git", "diff", "a b;echo nope"}},
		{"wezterm", []string{"start", "--cwd", "/repo with spaces", "--", "git", "diff", "a b;echo nope"}},
		{"wt.exe", []string{"-d", "/repo with spaces", "git", "diff", "a b;echo nope"}},
	} {
		_, args, err := desktopCommand("linux", tc.name, "/repo with spaces", []string{"git", "diff", "a b;echo nope"}, lookup)
		if err != nil || !reflect.DeepEqual(args, tc.want) {
			t.Fatalf("%s: %v, %v", tc.name, args, err)
		}
	}
	_, args, err := desktopCommand("darwin", "", "/repo's folder", []string{"git", "diff"}, lookup)
	if err != nil || !strings.Contains(args[1], `'\\''`) {
		t.Fatalf("AppleScript quoting = %v, %v", args, err)
	}
	if _, _, err := desktopCommand("linux", "", "/repo", nil, func(string) (string, error) { return "", errors.New("missing") }); err == nil {
		t.Fatal("missing terminal accepted")
	}
}

func TestPowerShellPreservesArgumentsAsLiterals(t *testing.T) {
	_, args, err := desktopCommand("windows", "powershell.exe", `C:\repo`, []string{"git", "diff", "a'; Remove-Item x"}, func(name string) (string, error) { return name, nil })
	want := []string{"-NoExit", "-NoLogo", "-Command", "& 'git' 'diff' 'a''; Remove-Item x'"}
	if err != nil || !reflect.DeepEqual(args, want) {
		t.Fatalf("PowerShell = %v, %v", args, err)
	}
}
