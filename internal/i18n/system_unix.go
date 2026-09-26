//go:build !darwin && !windows

package i18n

func systemLanguage() string { return "en" }
