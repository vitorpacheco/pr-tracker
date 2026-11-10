//! Resolves and runs CLIs for one unit of work, with its deadline and
//! cancellation, without touching the process-wide environment.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Carries per-application tool overrides, a deadline and cancellation flags
/// through a unit of work.
#[derive(Clone, Default)]
pub struct Ctx {
    paths: Arc<BTreeMap<String, String>>,
    deadline: Option<Instant>,
    cancel: Vec<Arc<AtomicBool>>,
}

impl Ctx {
    pub fn background() -> Self {
        Self::default()
    }

    /// Supplies per-application overrides without changing the process PATH.
    pub fn with_paths(&self, paths: &BTreeMap<String, String>) -> Self {
        Self {
            paths: Arc::new(paths.clone()),
            ..self.clone()
        }
    }

    /// Never extends a deadline the context already has.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;
        Self {
            deadline: Some(self.deadline.map_or(deadline, |own| own.min(deadline))),
            ..self.clone()
        }
    }

    pub fn with_cancel(&self, flag: Arc<AtomicBool>) -> Self {
        let mut next = self.clone();
        next.cancel.push(flag);
        next
    }

    pub fn err(&self) -> Option<Error> {
        if self.cancel.iter().any(|flag| flag.load(Ordering::SeqCst)) {
            return Some(Error::Canceled);
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Some(Error::DeadlineExceeded);
        }
        None
    }

    pub fn check(&self) -> Result<()> {
        self.err().map_or(Ok(()), Err)
    }
}

pub fn lookup(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    if let Some(explicit) = ctx.paths.get(name).filter(|path| !path.is_empty()) {
        return which::which(explicit).map_err(|_| Error::MissingTool(name.to_string()));
    }
    if let Ok(path) = which::which(name) {
        return Ok(path);
    }
    if cfg!(target_os = "macos") {
        // Finder does not necessarily inherit Homebrew's shell PATH.
        for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            if let Ok(path) = which::which(Path::new(dir).join(name)) {
                return Ok(path);
            }
        }
    }
    Err(Error::MissingTool(name.to_string()))
}

/// The PATH a child process gets, so CLIs find their own child tools in the
/// same desktop session.
pub fn child_path(ctx: &Ctx) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for path in ctx.paths.values() {
        let path = Path::new(path);
        if path.is_absolute()
            && let Some(dir) = path.parent()
        {
            dirs.push(dir.to_path_buf());
        }
    }
    if cfg!(target_os = "macos") {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    if let Some(current) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// A command for `program` that inherits the process environment, with PATH
/// extended for this context and no stdin.
pub fn command(ctx: &Ctx, program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", child_path(ctx)).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a GUI process must not flash a console per CLI call.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Runs `cmd` to completion, killing it when the context is canceled or its
/// deadline passes.
pub fn output(ctx: &Ctx, cmd: &mut Command) -> Result<Output> {
    ctx.check()?;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buffer);
            }
            buffer
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let mut wait = Duration::from_millis(2);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Some(err) = ctx.err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(Duration::from_millis(40));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_overrides_path_and_does_not_fall_back() {
        let _guard = crate::i18n::tests::env_lock();
        let executable = std::env::current_exe().unwrap();
        let mut paths =
            BTreeMap::from([("gh".to_string(), executable.to_string_lossy().into_owned())]);
        let found = lookup(&Ctx::background().with_paths(&paths), "gh").unwrap();
        assert_eq!(found, executable);
        paths.insert(
            "gh".to_string(),
            "/nonexistent/pr-tracker-test-cli".to_string(),
        );
        assert!(
            lookup(&Ctx::background().with_paths(&paths), "gh").is_err(),
            "invalid explicit path silently fell back"
        );
    }

    #[test]
    fn child_environment_does_not_mutate_process() {
        let _guard = crate::i18n::tests::env_lock();
        let before = std::env::var_os("PATH");
        let executable = std::env::current_exe().unwrap();
        let paths =
            BTreeMap::from([("git".to_string(), executable.to_string_lossy().into_owned())]);
        let path = child_path(&Ctx::background().with_paths(&paths));
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, executable.parent().unwrap());
        assert_eq!(
            std::env::var_os("PATH"),
            before,
            "child PATH was not isolated"
        );
    }

    #[test]
    fn deadline_kills_a_running_command() {
        let Ok(sleep) = which::which("sleep") else {
            return;
        };
        let ctx = Ctx::background().with_timeout(Duration::from_millis(50));
        let started = Instant::now();
        let err = output(&ctx, command(&ctx, &sleep).arg("5")).unwrap_err();
        assert_eq!(err, Error::DeadlineExceeded);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_is_reported_before_the_deadline() {
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = Ctx::background()
            .with_cancel(flag.clone())
            .with_timeout(Duration::from_secs(60));
        assert!(ctx.check().is_ok());
        flag.store(true, Ordering::SeqCst);
        assert_eq!(ctx.err(), Some(Error::Canceled));
    }
}
