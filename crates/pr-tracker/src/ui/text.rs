//! Wording shared by the views: translations, ages and status names.

use chrono::{DateTime, Utc};
use pr_tracker_core::i18n;

/// Translates an interface message.
pub fn t(language: &str, message: &str) -> String {
    i18n::text(language, message)
}

/// A short age such as "5 min. ago" or "há 3 h".
pub fn relative_age(time: Option<DateTime<Utc>>, now: DateTime<Utc>, language: &str) -> String {
    let Some(time) = time else {
        return "—".to_string();
    };
    let minutes = (now - time).num_minutes().max(0);
    let pt = language == "pt";
    match minutes {
        0 => if pt { "agora" } else { "now" }.to_string(),
        1..60 if pt => format!("há {minutes} min"),
        1..60 => format!("{minutes} min. ago"),
        60..1440 if pt => format!("há {} h", minutes / 60),
        60..1440 => format!("{} hr. ago", minutes / 60),
        1440..2880 => if pt { "ontem" } else { "yesterday" }.to_string(),
        _ if pt => format!("há {} dias", minutes / 1440),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// The name of a pipeline or review state.
pub fn status_label(language: &str, state: &str) -> String {
    let label = match state {
        "success" => "sucesso",
        "failure" => "falhou",
        "pending" => "em andamento",
        "canceled" => "cancelado",
        "approved" => "aprovado",
        "changes_requested" => "alterações solicitadas",
        "review_required" => "aguardando revisão",
        "commented" => "comentou",
        "dismissed" => "review descartado",
        other => other,
    };
    t(language, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn ages_are_short_and_localized() {
        let now = Utc::now();
        let ago = |minutes| Some(now - Duration::minutes(minutes));
        for (minutes, en, pt) in [
            (0, "now", "agora"),
            (5, "5 min. ago", "há 5 min"),
            (185, "3 hr. ago", "há 3 h"),
            (1500, "yesterday", "ontem"),
            (4400, "3 days ago", "há 3 dias"),
        ] {
            assert_eq!(relative_age(ago(minutes), now, "en"), en);
            assert_eq!(relative_age(ago(minutes), now, "pt"), pt);
        }
        assert_eq!(relative_age(None, now, "en"), "—");
        assert_eq!(
            relative_age(Some(now + Duration::minutes(9)), now, "en"),
            "now",
            "clock skew"
        );
    }

    #[test]
    fn status_names_are_translated_and_unknown_ones_pass_through() {
        assert_eq!(
            status_label("pt", "changes_requested"),
            "alterações solicitadas"
        );
        assert_eq!(status_label("en", "changes_requested"), "changes requested");
        assert_eq!(status_label("en", "skipped"), "skipped");
    }
}
