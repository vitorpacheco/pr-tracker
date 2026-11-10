//! Resolves the interface language and translates application messages.
//! Portuguese source messages are the stable catalog keys.

use std::collections::HashMap;
use std::sync::OnceLock;

fn english() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(include_str!("i18n/en.json")).expect("en.json is a string map"))
}

/// Reports whether a persisted preference is valid. Empty values keep
/// configuration files written before translations existed working.
pub fn supported(preference: &str) -> bool {
    matches!(preference, "" | "system" | "en" | "pt")
}

/// Reduces OS locale identifiers (pt_BR.UTF-8, pt-PT) to a supported language.
pub fn match_locale(locale: &str) -> &'static str {
    let locale = locale.trim().to_lowercase();
    match locale.split(['_', '-', '.', '@']).find(|part| !part.is_empty()) {
        Some("pt") => "pt",
        _ => "en",
    }
}

fn native_language() -> &'static str {
    static NATIVE: OnceLock<&'static str> = OnceLock::new();
    NATIVE.get_or_init(|| match_locale(&system_language()))
}

#[cfg(any(target_os = "macos", windows))]
fn system_language() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en".to_string())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_language() -> String {
    "en".to_string()
}

/// Honors an explicit preference, otherwise the computer's locale. POSIX
/// locale overrides take precedence over the native desktop setting.
pub fn resolve(preference: &str) -> &'static str {
    if !preference.is_empty() && preference != "system" {
        return match_locale(preference);
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            return message_language(&value);
        }
    }
    native_language()
}

/// Translates an application-owned message, never provider or user content.
pub fn text(language: &str, message: &str) -> String {
    if language == "pt" {
        return message.to_string();
    }
    english().get(message).cloned().unwrap_or_else(|| message.to_string())
}

/// GNU message preferences override a non-C POSIX locale. Unsupported entries
/// fall through to the next preferred language, then the effective locale.
fn message_language(locale: &str) -> &'static str {
    let base = locale.split('.').next().unwrap_or_default().to_lowercase();
    if base != "c" && base != "posix" {
        let preferred = std::env::var("LANGUAGE").unwrap_or_default();
        for candidate in preferred.split(':') {
            let tag = candidate.trim().to_lowercase();
            match tag.split(['.', '_', '-']).next() {
                Some("pt") => return "pt",
                Some("en") => return "en",
                _ => {}
            }
        }
    }
    match_locale(locale)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::Error;
    use crate::msg;
    use std::sync::{Mutex, MutexGuard};

    /// Tests that touch process environment variables hold this lock.
    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_env(key: &str, value: &str) {
        // SAFETY: every test that reads or writes the environment holds env_lock.
        unsafe { std::env::set_var(key, value) }
    }

    #[test]
    fn locale_variants() {
        for (locale, want) in [
            ("pt_BR.UTF-8", "pt"),
            ("pt-PT", "pt"),
            ("PT_br@latin", "pt"),
            ("en_GB.UTF-8", "en"),
            ("C", "en"),
            ("POSIX", "en"),
            ("de_DE", "en"),
            ("", "en"),
        ] {
            assert_eq!(match_locale(locale), want, "match_locale({locale:?})");
        }
    }

    #[test]
    fn preference_and_environment_precedence() {
        let _guard = env_lock();
        for (preference, all, messages, lang, want) in [
            ("system", "", "", "pt_BR.UTF-8", "pt"),
            ("", "", "pt-PT", "en_US.UTF-8", "pt"),
            ("system", "en_US.UTF-8", "pt_BR", "pt_BR", "en"),
            ("system", "C", "pt_BR", "pt_BR", "en"),
            ("system", "fr_FR", "pt_BR", "pt_BR", "en"),
            ("en", "pt_BR", "pt_BR", "pt_BR", "en"),
            ("pt", "C", "en_US", "en_US", "pt"),
        ] {
            set_env("LANGUAGE", "");
            set_env("LC_ALL", all);
            set_env("LC_MESSAGES", messages);
            set_env("LANG", lang);
            assert_eq!(resolve(preference), want, "{preference:?} {all:?} {messages:?} {lang:?}");
        }
    }

    #[test]
    fn catalog_preserves_formatting() {
        let verbs = regex::Regex::new(r"%[-+ #0-9.]*[a-zA-Z]").unwrap();
        let found = |text: &str| verbs.find_iter(text).map(|m| m.as_str().to_string()).collect::<Vec<_>>();
        for (source, translated) in english() {
            assert!(!translated.is_empty(), "empty translation for {source:?}");
            assert_eq!(found(source), found(translated), "format verbs differ: {source:?} => {translated:?}");
        }
        assert_eq!(text("pt", "Configurações"), "Configurações");
        assert_eq!(text("en", "Configurações"), "Settings");
        assert_eq!(text("en", "user supplied content"), "user supplied content");
    }

    #[test]
    fn language_priority_list() {
        let _guard = env_lock();
        set_env("LC_ALL", "en_US.UTF-8");
        set_env("LANGUAGE", "de:pt_BR:en");
        assert_eq!(resolve("system"), "pt", "message preference ignored");
        set_env("LC_ALL", "C");
        assert_eq!(resolve("system"), "en", "C locale must ignore LANGUAGE");
        set_env("LANGUAGE", "");
    }

    #[test]
    fn localized_errors_preserve_identity_and_arguments() {
        let sentinel = msg!("aplicação encerrada");
        let err = msg!("resposta inválida do gh para %s: %w", "org/Configurações", Error::external("upstream failure"));
        assert_eq!(err.text("en"), "invalid gh response for org/Configurações: upstream failure");
        assert_eq!(err.text("pt"), err.to_string());
        let wrapped = msg!("lendo %s: %w", "x", sentinel.clone());
        assert!(wrapped.is("aplicação encerrada"), "wrapped sentinel lost");
        let external = Error::external("configurações salvas");
        assert_eq!(external.text("en"), "configurações salvas", "external output changed");
        let nested = msg!("tea api %s: %w", "/repos", msg!("resposta inválida do tea: %w", Error::external("upstream failure")));
        assert_eq!(nested.text("en"), "tea api /repos: invalid tea response: upstream failure");
    }
}
