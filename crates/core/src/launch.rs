//! Opens terminals, folders and browsers. STUB: replaced by the port of internal/launch.

use std::path::Path;

use crate::error::Result;
use crate::toolchain::Ctx;

/// Where a command opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Herdr,
    Tmux,
    /// No multiplexer applies: open a standalone terminal window.
    Window,
}

/// Turns the configured `terminal` setting into a concrete mode.
pub fn resolve(_setting: &str) -> Mode {
    todo!()
}

/// Runs `argv` (or an interactive shell when empty) in `dir`: in a new herdr
/// tab or tmux window, or else in a standalone terminal window, where
/// `desktop_terminal` optionally names the terminal program.
pub fn open(_ctx: &Ctx, _mode: Mode, _desktop_terminal: &str, _dir: &Path, _label: &str, _argv: &[String]) -> Result<()> {
    todo!()
}

/// Joins argv into a POSIX shell command line.
pub fn quote(_argv: &[String]) -> String {
    todo!()
}

/// Opens a URL with the platform opener.
pub fn browser(_url: &str) -> Result<()> {
    todo!()
}

/// Opens a directory in the platform file manager.
pub fn folder(_dir: &Path) -> Result<()> {
    todo!()
}
