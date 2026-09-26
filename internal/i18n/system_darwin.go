package i18n

import (
	"context"
	"os/exec"
	"strings"
	"time"
)

func systemLanguage() string {
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	data, err := exec.CommandContext(ctx, "defaults", "read", "-g", "AppleLanguages").Output()
	if err != nil {
		return "en"
	}
	languages := strings.FieldsFunc(string(data), func(r rune) bool { return strings.ContainsRune("()\", \n\t", r) })
	if len(languages) > 0 {
		return languages[0]
	}
	return "en"
}
