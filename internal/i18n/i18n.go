// Package i18n resolves interface languages and shares a message catalog with
// the desktop frontend. Portuguese source messages are stable catalog keys.
package i18n

import (
	"embed"
	"encoding/json"
	"os"
	"strings"
	"sync"
)

//go:embed en.json
var catalogs embed.FS

var english = func() map[string]string {
	data, err := catalogs.ReadFile("en.json")
	if err != nil {
		panic(err)
	}
	var messages map[string]string
	if err := json.Unmarshal(data, &messages); err != nil {
		panic(err)
	}
	return messages
}()

// Supported reports whether a persisted preference is valid. Empty values
// preserve compatibility with configuration files created before i18n.
func Supported(preference string) bool {
	switch preference {
	case "", "system", "en", "pt":
		return true
	}
	return false
}

// Match reduces OS locale identifiers (pt_BR.UTF-8, pt-PT) to a supported language.
func Match(locale string) string {
	locale = strings.ToLower(strings.TrimSpace(locale))
	base := strings.FieldsFunc(locale, func(r rune) bool { return strings.ContainsRune("_-.@", r) })
	if len(base) > 0 && base[0] == "pt" {
		return "pt"
	}
	return "en"
}

var nativeLanguage = sync.OnceValue(systemLanguage)

// Resolve honors an explicit preference, otherwise the computer's locale.
// POSIX locale overrides take precedence over the native desktop setting.
func Resolve(preference string) string {
	if preference != "" && preference != "system" {
		return Match(preference)
	}
	for _, key := range []string{"LC_ALL", "LC_MESSAGES", "LANG"} {
		if value := os.Getenv(key); value != "" {
			return messageLanguage(value)
		}
	}
	return Match(nativeLanguage())
}

// Text translates an application-owned message, never provider/user content.
func Text(language, message string) string {
	if language == "pt" {
		return message
	}
	if translated, ok := english[message]; ok {
		return translated
	}
	return message
}

// GNU message preferences override a non-C POSIX locale. Unsupported entries
// fall through to the next preferred language, then the effective locale.
func messageLanguage(locale string) string {
	base := strings.ToLower(strings.Split(locale, ".")[0])
	if base != "c" && base != "posix" {
		for _, candidate := range strings.Split(os.Getenv("LANGUAGE"), ":") {
			tag := strings.ToLower(strings.TrimSpace(candidate))
			tag = strings.Split(strings.Split(strings.Split(tag, ".")[0], "_")[0], "-")[0]
			if tag == "pt" || tag == "en" {
				return tag
			}
		}
	}
	return Match(locale)
}
