//! Providers, worktrees, cache and application state shared by pr-tracker's
//! command line and window.

pub mod app;
pub mod cache;
pub mod config;
pub mod error;
pub mod gitops;
pub mod i18n;
pub mod launch;
pub mod provider;
pub mod theme;
pub mod toolchain;

pub use error::{Error, Result};
