package i18n

import (
	"errors"
	"fmt"
	"regexp"
	"slices"
	"testing"
)

func TestLocaleVariants(t *testing.T) {
	for _, tc := range []struct{ locale, want string }{
		{"pt_BR.UTF-8", "pt"}, {"pt-PT", "pt"}, {"PT_br@latin", "pt"},
		{"en_GB.UTF-8", "en"}, {"C", "en"}, {"POSIX", "en"}, {"de_DE", "en"}, {"", "en"},
	} {
		t.Run(tc.locale, func(t *testing.T) {
			if got := Match(tc.locale); got != tc.want {
				t.Fatalf("Match(%q) = %s", tc.locale, got)
			}
		})
	}
}

func TestPreferenceAndEnvironmentPrecedence(t *testing.T) {
	for _, tc := range []struct{ preference, all, messages, lang, want string }{
		{"system", "", "", "pt_BR.UTF-8", "pt"},
		{"", "", "pt-PT", "en_US.UTF-8", "pt"},
		{"system", "en_US.UTF-8", "pt_BR", "pt_BR", "en"},
		{"system", "C", "pt_BR", "pt_BR", "en"},
		{"system", "fr_FR", "pt_BR", "pt_BR", "en"},
		{"en", "pt_BR", "pt_BR", "pt_BR", "en"},
		{"pt", "C", "en_US", "en_US", "pt"},
	} {
		t.Run(fmt.Sprint(tc), func(t *testing.T) {
			t.Setenv("LANGUAGE", "")
			t.Setenv("LC_ALL", tc.all)
			t.Setenv("LC_MESSAGES", tc.messages)
			t.Setenv("LANG", tc.lang)
			if got := Resolve(tc.preference); got != tc.want {
				t.Fatalf("Resolve = %s, want %s", got, tc.want)
			}
		})
	}
}

func TestCatalogPreservesFormatting(t *testing.T) {
	verbs := regexp.MustCompile(`%[-+ #0-9.]*[a-zA-Z]`)
	for source, translated := range english {
		if translated == "" {
			t.Errorf("empty translation for %q", source)
		}
		if !slices.Equal(verbs.FindAllString(source, -1), verbs.FindAllString(translated, -1)) {
			t.Errorf("format verbs differ: %q => %q", source, translated)
		}
	}
	if Text("pt", "Configurações") != "Configurações" || Text("en", "Configurações") != "Settings" {
		t.Fatal("incorrect interface translation")
	}
	if Text("en", "user supplied content") != "user supplied content" {
		t.Fatal("unknown message changed")
	}
}

func TestLanguagePriorityList(t *testing.T) {
	t.Setenv("LC_ALL", "en_US.UTF-8")
	t.Setenv("LANGUAGE", "de:pt_BR:en")
	if Resolve("system") != "pt" {
		t.Fatal("message preference ignored")
	}
	t.Setenv("LC_ALL", "C")
	if Resolve("system") != "en" {
		t.Fatal("C locale must ignore LANGUAGE")
	}
}

func TestLocalizedErrorsPreserveIdentityAndArguments(t *testing.T) {
	sentinel := errors.New("upstream failure")
	err := Errorf("resposta inválida do gh para %s: %w", "org/Configurações", sentinel)
	if !errors.Is(err, sentinel) {
		t.Fatal("wrapped sentinel lost")
	}
	if got := ErrorText("en", err); got != "invalid gh response for org/Configurações: upstream failure" {
		t.Fatalf("translated error = %q", got)
	}
	if ErrorText("pt", err) != err.Error() {
		t.Fatal("Portuguese source message changed")
	}
	external := errors.New("configurações salvas")
	if ErrorText("en", external) != external.Error() {
		t.Fatal("external output changed")
	}
	nested := Errorf("tea api %s: %w", "/repos", Errorf("resposta inválida do tea: %w", sentinel))
	if got := ErrorText("en", nested); got != "tea api /repos: invalid tea response: upstream failure" {
		t.Fatal(got)
	}
}
