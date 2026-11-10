//! Errors keep their message format and arguments apart, so an interface can
//! translate them without touching paths, repository names or CLI output.

use std::fmt;

use crate::i18n;

pub type Result<T> = std::result::Result<T, Error>;

/// One argument of a [`Error::Message`].
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Str(String),
    Int(i64),
    Err(Box<Error>),
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Str(value.to_string())
    }
}
impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Str(value)
    }
}
impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Str(value.clone())
    }
}
impl From<i64> for Arg {
    fn from(value: i64) -> Self {
        Arg::Int(value)
    }
}
impl From<usize> for Arg {
    fn from(value: usize) -> Self {
        Arg::Int(value as i64)
    }
}
impl From<Error> for Arg {
    fn from(value: Error) -> Self {
        Arg::Err(Box::new(value))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// An application message. `format` is the Portuguese catalog key and uses
    /// the verbs `%s`, `%d`, `%q`, `%v` and `%w`.
    Message { format: String, args: Vec<Arg> },
    /// Text owned by something else (a CLI, the OS). Never translated.
    External(String),
    /// A required CLI is not installed.
    MissingTool(String),
    /// The caller gave up on the operation.
    Canceled,
    /// The operation ran past its deadline.
    DeadlineExceeded,
    /// Independent failures of one operation, reported together.
    Joined(Vec<Error>),
}

/// Builds an [`Error::Message`]: `msg!("lendo %s: %w", path, err)`.
#[macro_export]
macro_rules! msg {
    ($format:expr $(, $arg:expr)* $(,)?) => {
        $crate::error::Error::Message {
            format: ::std::string::String::from($format),
            args: ::std::vec![$($crate::error::Arg::from($arg)),*],
        }
    };
}

impl Error {
    pub fn external(text: impl Into<String>) -> Self {
        Error::External(text.into())
    }

    /// One error for several: `None` when there are none, the error itself
    /// when there is one.
    pub fn join(mut errors: Vec<Error>) -> Option<Error> {
        match errors.len() {
            0 => None,
            1 => errors.pop(),
            _ => Some(Error::Joined(errors)),
        }
    }

    /// Reports whether the chain contains `target`, as when a caller's
    /// cancellation surfaces wrapped in a message.
    pub fn contains(&self, target: &Error) -> bool {
        if self == target {
            return true;
        }
        match self {
            Error::Message { args, .. } => args.iter().any(|arg| matches!(arg, Arg::Err(inner) if inner.contains(target))),
            Error::Joined(errors) => errors.iter().any(|err| err.contains(target)),
            _ => false,
        }
    }

    /// Reports whether this error, or one it wraps, is the application message
    /// `format`. Sentinels are compared by their catalog key.
    pub fn is(&self, format: &str) -> bool {
        match self {
            Error::Message { format: own, args } => {
                own == format || args.iter().any(|arg| matches!(arg, Arg::Err(inner) if inner.is(format)))
            }
            Error::Joined(errors) => errors.iter().any(|err| err.is(format)),
            _ => false,
        }
    }

    /// Reports whether a required CLI was missing anywhere in the chain.
    pub fn is_missing_tool(&self) -> bool {
        match self {
            Error::MissingTool(_) => true,
            Error::Message { args, .. } => args.iter().any(|arg| matches!(arg, Arg::Err(inner) if inner.is_missing_tool())),
            Error::Joined(errors) => errors.iter().any(Error::is_missing_tool),
            _ => false,
        }
    }

    /// The message in `language` ("pt" or "en").
    pub fn text(&self, language: &str) -> String {
        match self {
            Error::Message { format, args } => {
                let rendered: Vec<Rendered> = args
                    .iter()
                    .map(|arg| match arg {
                        Arg::Str(text) => Rendered::Str(text.clone()),
                        Arg::Int(n) => Rendered::Int(*n),
                        Arg::Err(inner) => Rendered::Str(inner.text(language)),
                    })
                    .collect();
                sprintf(&i18n::text(language, format), &rendered)
            }
            Error::External(text) => text.clone(),
            Error::MissingTool(tool) => sprintf(
                &i18n::text(language, "%s não encontrado no PATH. Instale: %s"),
                &[Rendered::Str(tool.clone()), Rendered::Str(i18n::text(language, crate::provider::install_hint(tool)))],
            ),
            Error::Canceled => i18n::text(language, "operação cancelada"),
            Error::DeadlineExceeded => i18n::text(language, "tempo limite excedido"),
            Error::Joined(errors) => errors.iter().map(|err| err.text(language)).collect::<Vec<_>>().join("\n"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text("pt"))
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::External(err.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::External(err.to_string())
    }
}
impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::External(err.to_string())
    }
}

pub enum Rendered {
    Str(String),
    Int(i64),
}

/// Formats the small subset of Go's verbs the message catalog uses.
pub fn sprintf(format: &str, args: &[Rendered]) -> String {
    let mut out = String::with_capacity(format.len());
    let mut next = args.iter();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(verb @ ('s' | 'd' | 'v' | 'w' | 'q')) => match next.next() {
                Some(Rendered::Str(text)) if verb == 'q' => out.push_str(&format!("{text:?}")),
                Some(Rendered::Str(text)) => out.push_str(text),
                Some(Rendered::Int(n)) => out.push_str(&n.to_string()),
                None => {
                    out.push_str("%!");
                    out.push(verb);
                    out.push_str("(MISSING)");
                }
            },
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Formats a translated interface string with plain arguments.
pub fn format(language: &str, message: &str, args: &[Rendered]) -> String {
    sprintf(&i18n::text(language, message), args)
}
