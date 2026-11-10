use std::path::PathBuf;
use std::time::Duration;

use super::session::{ERR_CLOSED, Session};
use crate::error::Result;
use crate::msg;
use crate::provider;
use crate::toolchain::{self, Ctx};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolStatus {
    pub name: &'static str,
    /// `None` when the tool is not installed.
    pub path: Option<PathBuf>,
    /// Where to get the tool; a catalog key.
    pub hint: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceStatus {
    pub name: String,
    pub disabled: bool,
    /// The authentication failure, already in the interface language.
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diagnostics {
    pub tools: Vec<ToolStatus>,
    pub instances: Vec<InstanceStatus>,
}

impl Session {
    /// Checks the installed CLIs and each instance's authentication.
    pub fn diagnose(&self, ctx: &Ctx) -> Result<Diagnostics> {
        let (application, _worker) = self.begin_work().ok_or_else(|| msg!(ERR_CLOSED))?;
        let cfg = application.config();
        let ctx = self.work(ctx, Duration::from_secs(30)).with_paths(&cfg.tool_paths);
        let language = crate::i18n::resolve(&cfg.language);
        let mut result = Diagnostics::default();
        for name in ["git", "gh", "glab", "tea", "hunk"] {
            result.tools.push(ToolStatus { name, path: toolchain::lookup(&ctx, name).ok(), hint: provider::install_hint(name) });
        }
        for instance in &cfg.instances {
            let error = match instance.disabled {
                true => String::new(),
                false => (application.hooks.client)(instance, &cfg.repos).auth_status(&ctx).err().map(|err| err.text(language)).unwrap_or_default(),
            };
            result.instances.push(InstanceStatus { name: instance.name.clone(), disabled: instance.disabled, error });
        }
        ctx.check()?;
        Ok(result)
    }
}
