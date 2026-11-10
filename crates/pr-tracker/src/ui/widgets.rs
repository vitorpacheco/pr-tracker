//! Small pieces the views share.

use rune::{Color, El, Icon, TextStyle, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Spinner};

use super::state::App;
use super::theme::Look;

pub static REFRESH: Icon = Icon {
    name: "prt-refresh",
    view: 16.0,
    body: r#"<path d="M13 8a5 5 0 1 1-1.5-3.55M13 2.5v3h-3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};
pub static GEAR: Icon = Icon {
    name: "prt-gear",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="2" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 1.75v1.6M8 12.65v1.6M1.75 8h1.6M12.65 8h1.6M3.58 3.58l1.13 1.13M11.29 11.29l1.13 1.13M3.58 12.42l1.13-1.13M11.29 4.71l1.13-1.13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};
pub static COMMAND: Icon = Icon {
    name: "prt-command",
    view: 16.0,
    body: r#"<path d="M6 6h4v4H6zM6 6V4.5A1.5 1.5 0 1 0 4.5 6H6zM10 6V4.5A1.5 1.5 0 1 1 11.5 6H10zM6 10v1.5A1.5 1.5 0 1 1 4.5 10H6zM10 10v1.5a1.5 1.5 0 1 0 1.5-1.5H10z" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round" fill="none"/>"#,
};
pub static BRANCH: Icon = Icon {
    name: "prt-branch",
    view: 16.0,
    body: r#"<circle cx="4.5" cy="3.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="4.5" cy="12.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="5.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5v6M11.5 7c0 2.5-3 2.5-7 3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};
pub static ISSUE: Icon = Icon {
    name: "prt-issue",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="8" r="1.5" fill="currentColor"/>"#,
};
pub static COMMENT: Icon = Icon {
    name: "prt-comment",
    view: 16.0,
    body: r#"<path d="M3 3.5h10v7H7.5L4.5 13v-2.5H3z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#,
};

/// A small rounded label.
pub fn pill(label: impl Into<String>, ink: Color, look: &Look) -> El {
    row()
        .px(7.0)
        .h(18.0)
        .center_y()
        .radius(9.0)
        .fill(ink.alpha(0.14))
        .border(1.0, ink.alpha(0.4))
        .child(text(label.into()).font(look.kit.ty.small_medium).color(ink))
}

/// A keyboard key cap.
pub fn kbd(key: impl Into<String>, look: &Look) -> El {
    row()
        .px(5.0)
        .h(18.0)
        .min_w(18.0)
        .center()
        .radius(4.0)
        .fill(look.raised)
        .border(1.0, look.border)
        .child(text(key.into()).font(look.kit.ty.small).color(look.muted))
}

/// A spinner beside what is in progress.
pub fn activity(label: impl Into<String>, look: &Look) -> El {
    row()
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .center_y()
        .radius(look.kit.radius.md)
        .fill(look.raised)
        .child(Spinner::new().size(14.0).color(look.accent).build(&look.kit))
        .child(text(label.into()).font(look.kit.ty.ui).color(look.muted).truncate().grow(1.0))
}

pub enum BannerKind {
    Error,
    Notice,
}

/// A message across the pane, optionally dismissible.
pub fn banner(key: &str, message: impl Into<String>, kind: BannerKind, dismiss: Option<fn(&mut App)>, look: &Look) -> El {
    let ink = match kind {
        BannerKind::Error => look.red,
        BannerKind::Notice => look.accent,
    };
    let mut banner = row()
        .key(key.to_string())
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .items_start()
        .radius(look.kit.radius.md)
        .fill(ink.alpha(0.1))
        .border(1.0, ink.alpha(0.35))
        .child(text(message.into()).font(look.kit.ty.ui).color(look.text).wrap().selectable(true).grow(1.0));
    if let Some(dismiss) = dismiss {
        banner = banner.child(
            IconButton::new(look.kit.glyph.close)
                .variant(IconButtonVariant::Ghost)
                .label("×")
                .on_press(dismiss)
                .build(&look.kit)
                .key(format!("{key}-dismiss")),
        );
    }
    banner
}

/// A small uppercase caption above a section.
pub fn eyebrow(label: impl Into<String>, look: &Look) -> El {
    text(label.into().to_uppercase()).font(TextStyle { tracking: 0.6, ..look.kit.ty.small_medium }).color(look.muted)
}

/// A titled block in the inspector.
pub fn section(title: impl Into<String>, look: &Look) -> El {
    col().w_full().gap(8.0).pt(14.0).child(text(title.into()).font(look.kit.ty.ui_medium).color(look.text))
}

/// A one-pixel horizontal rule.
pub fn rule(look: &Look) -> El {
    el().w_full().h(1.0).fill(look.border)
}

/// An icon tinted with `ink`.
pub fn tinted(glyph: &'static Icon, size: f32, ink: Color) -> El {
    icon(glyph, size).color(ink)
}

/// The initials of an author in a rounded square.
pub fn avatar(name: &str, look: &Look) -> El {
    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
    row()
        .size(22.0, 22.0)
        .radius(6.0)
        .center()
        .fill(look.accent.alpha(0.16))
        .border(1.0, look.accent.alpha(0.35))
        .child(text(initials).font(look.kit.ty.small_medium).color(look.accent))
}
