use std::path::Path;

use super::{Client, Item, MergeOptions, Thread};
use crate::config::Instance;
use crate::error::Result;
use crate::toolchain::Ctx;

pub(super) struct GitLab {
    instance: Instance,
    #[allow(dead_code)]
    tracked: Vec<String>,
}

impl GitLab {
    pub(super) fn new(instance: Instance, tracked: Vec<String>) -> Self {
        Self { instance, tracked }
    }
}

impl Client for GitLab {
    fn instance(&self) -> &Instance {
        &self.instance
    }
    fn tool(&self) -> &'static str {
        "glab"
    }
    fn list(&self, _: &Ctx) -> Result<Vec<Item>> {
        todo!()
    }
    fn thread(&self, _: &Ctx, _: &Item) -> Result<Thread> {
        todo!()
    }
    fn add_comment(&self, _: &Ctx, _: &Item, _: &str) -> Result<()> {
        todo!()
    }
    fn approve(&self, _: &Ctx, _: &Item) -> Result<()> {
        todo!()
    }
    fn merge(&self, _: &Ctx, _: &Item, _: &MergeOptions) -> Result<()> {
        todo!()
    }
    fn close(&self, _: &Ctx, _: &Item) -> Result<()> {
        todo!()
    }
    fn set_draft(&self, _: &Ctx, _: &Item, _: bool) -> Result<()> {
        todo!()
    }
    fn update_branch(&self, _: &Ctx, _: &Item) -> Result<()> {
        todo!()
    }
    fn checkout(&self, _: &Ctx, _: &Item, _: &Path) -> Result<()> {
        todo!()
    }
    fn head_ref(&self, _: &Item) -> String {
        todo!()
    }
    fn auth_status(&self, _: &Ctx) -> Result<()> {
        todo!()
    }
}
