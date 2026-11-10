//! Clones and worktrees. STUB: replaced by the port of internal/gitops.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::Result;
use crate::provider::{Client, Item};
use crate::toolchain::Ctx;

/// Catalog key of the error for a worktree with uncommitted changes.
pub const ERR_DIRTY: &str = "worktree tem alterações locais";

/// Runs git in `dir` and returns its trimmed stdout.
pub fn git(_ctx: &Ctx, _dir: &Path, _args: &[&str]) -> Result<String> {
    todo!()
}

/// The stable eight-character hash of host, repository and number.
pub fn hash(_pr: &Item) -> String {
    todo!()
}

pub fn worktree_path(_cfg: &Config, _pr: &Item) -> Result<PathBuf> {
    todo!()
}

/// The worktree directory of a PR, when it exists on disk.
pub fn worktree_exists(_cfg: &Config, _pr: &Item) -> Option<PathBuf> {
    todo!()
}

pub fn local_branch(_pr: &Item) -> String {
    todo!()
}

/// The local clone of the PR's repository and the remote that points at it.
pub fn resolve_clone(_ctx: &Ctx, _cfg: &Config, _pr: &Item) -> Option<(PathBuf, String)> {
    todo!()
}

pub fn validate_clone(_ctx: &Ctx, _path: &Path) -> Result<()> {
    todo!()
}

/// Creates or updates the PR's worktree and returns its directory.
pub fn create_worktree(_ctx: &Ctx, _cfg: &Config, _client: &dyn Client, _pr: &Item, _clone: &Path, _remote: &str) -> Result<PathBuf> {
    todo!()
}

pub fn push_worktree(_ctx: &Ctx, _worktree: &Path, _pr: &Item) -> Result<()> {
    todo!()
}

pub fn remove_worktree(_ctx: &Ctx, _cfg: &Config, _pr: &Item, _force: bool) -> Result<()> {
    todo!()
}
