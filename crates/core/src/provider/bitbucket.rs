use std::path::Path;

use super::{Client, ERR_NOT_SUPPORTED, Item, MergeOptions, Thread};
use crate::config::Instance;
use crate::error::{Error, Result};
use crate::msg;
use crate::toolchain::Ctx;

/// A placeholder. Atlassian ships no official Bitbucket CLI (acli covers
/// Jira/admin only); see docs/bitbucket.md for the options found.
pub(super) struct Bitbucket {
    instance: Instance,
}

impl Bitbucket {
    pub(super) fn new(instance: Instance) -> Self {
        Self { instance }
    }

    fn err(&self) -> Error {
        msg!(
            "%w: Bitbucket (%s) — veja docs/bitbucket.md",
            msg!(ERR_NOT_SUPPORTED),
            &self.instance.host
        )
    }
}

impl Client for Bitbucket {
    fn instance(&self) -> &Instance {
        &self.instance
    }
    fn tool(&self) -> &'static str {
        "bkt"
    }
    fn list(&self, _: &Ctx) -> Result<Vec<Item>> {
        Err(self.err())
    }
    fn thread(&self, _: &Ctx, _: &Item) -> Result<Thread> {
        Err(self.err())
    }
    fn add_comment(&self, _: &Ctx, _: &Item, _: &str) -> Result<()> {
        Err(self.err())
    }
    fn approve(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(self.err())
    }
    fn merge(&self, _: &Ctx, _: &Item, _: &MergeOptions) -> Result<()> {
        Err(self.err())
    }
    fn close(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(self.err())
    }
    fn set_draft(&self, _: &Ctx, _: &Item, _: bool) -> Result<()> {
        Err(self.err())
    }
    fn update_branch(&self, _: &Ctx, _: &Item) -> Result<()> {
        Err(self.err())
    }
    fn checkout(&self, _: &Ctx, _: &Item, _: &Path) -> Result<()> {
        Err(self.err())
    }
    fn head_ref(&self, _: &Item) -> String {
        String::new()
    }
    fn auth_status(&self, _: &Ctx) -> Result<()> {
        Err(self.err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Provider;

    #[test]
    fn every_operation_reports_the_missing_integration() {
        let client = Bitbucket::new(Instance {
            name: "bb".into(),
            provider: Provider::Bitbucket,
            host: "bitbucket.org".into(),
            ..Instance::default()
        });
        let err = client.list(&Ctx::background()).unwrap_err();
        assert!(err.is(ERR_NOT_SUPPORTED));
        assert_eq!(
            err.text("en"),
            "provider not supported yet: Bitbucket (bitbucket.org) — see docs/bitbucket.md"
        );
    }
}
