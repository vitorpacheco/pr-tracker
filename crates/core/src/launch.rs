//! Opens shells and tools in a directory, using herdr or tmux when available
//! and falling back to a standalone terminal window, and hands URLs and
//! folders to the platform opener.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::error::{Error, Result};
use crate::msg;
use crate::toolchain::{self, Ctx};

/// Where a command opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Herdr,
    Tmux,
    /// No multiplexer applies: open a standalone terminal window.
    Window,
}

fn env_is(name: &str, accept: impl Fn(&OsStr) -> bool) -> bool {
    std::env::var_os(name).is_some_and(|value| accept(&value))
}

fn inside_tmux() -> bool {
    env_is("TMUX", |value| !value.is_empty())
}

fn herdr_installed() -> bool {
    which::which("herdr").is_ok()
}

/// Turns the configured `terminal` setting into a concrete mode.
pub fn resolve(setting: &str) -> Mode {
    match setting {
        "herdr" if herdr_installed() => Mode::Herdr,
        "tmux" if inside_tmux() => Mode::Tmux,
        "herdr" | "tmux" | "inline" => Mode::Window,
        // auto
        _ if env_is("HERDR_ENV", |value| value == "1") && herdr_installed() => Mode::Herdr,
        _ if inside_tmux() => Mode::Tmux,
        _ => Mode::Window,
    }
}

/// Runs `argv` (or an interactive shell when empty) in `dir`: in a new herdr
/// tab or tmux window, or else in a standalone terminal window, where
/// `desktop_terminal` optionally names the terminal program.
pub fn open(
    ctx: &Ctx,
    mode: Mode,
    desktop_terminal: &str,
    dir: &Path,
    label: &str,
    argv: &[String],
) -> Result<()> {
    match mode {
        Mode::Herdr => open_herdr(ctx, dir, label, argv),
        Mode::Tmux => {
            let mut args: Vec<&OsStr> = vec![
                "new-window".as_ref(),
                "-c".as_ref(),
                dir.as_os_str(),
                "-n".as_ref(),
            ];
            args.push(label.as_ref());
            let command = quote(argv);
            if !argv.is_empty() {
                args.push(command.as_ref());
            }
            let output = run(ctx, "tmux", &args)?;
            if !output.status.success() {
                return Err(Error::external(format!("tmux: {}", combined(&output))));
            }
            Ok(())
        }
        Mode::Window => {
            // The terminal does not share this context's tool overrides, so
            // it gets the program as an absolute path.
            let mut argv = argv.to_vec();
            if let Some(program) = argv.first_mut() {
                *program = toolchain::lookup(ctx, program)?
                    .to_string_lossy()
                    .into_owned();
            }
            open_window(ctx, desktop_terminal, dir, &argv)
        }
    }
}

/// Runs a multiplexer CLI to completion under the context's deadline.
fn run(ctx: &Ctx, tool: &str, args: &[&OsStr]) -> Result<Output> {
    let path = toolchain::lookup(ctx, tool)?;
    let mut cmd = toolchain::command(ctx, &path);
    cmd.args(args);
    toolchain::output(ctx, &mut cmd).map_err(|err| match err {
        Error::Canceled | Error::DeadlineExceeded => err,
        other => Error::external(format!("{tool}: {other}")),
    })
}

fn combined(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.trim().to_string()
}

fn open_herdr(ctx: &Ctx, dir: &Path, label: &str, argv: &[String]) -> Result<()> {
    let create: [&OsStr; 7] = [
        "tab".as_ref(),
        "create".as_ref(),
        "--cwd".as_ref(),
        dir.as_os_str(),
        "--label".as_ref(),
        label.as_ref(),
        "--focus".as_ref(),
    ];
    let output = run(ctx, "herdr", &create)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::external(format!("herdr: {}", stderr.trim())));
    }
    if argv.is_empty() {
        return Ok(());
    }
    let response = String::from_utf8_lossy(&output.stdout);
    let response = response.trim();
    let pane = serde_json::from_str::<serde_json::Value>(response)
        .ok()
        .and_then(|value| {
            value
                .pointer("/result/root_pane/pane_id")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|pane| !pane.is_empty())
        .ok_or_else(|| msg!("herdr: resposta inesperada de tab create: %s", response))?;
    let command = quote(argv);
    let output = run(
        ctx,
        "herdr",
        &[
            "pane".as_ref(),
            "run".as_ref(),
            pane.as_ref(),
            command.as_ref(),
        ],
    )?;
    if !output.status.success() {
        return Err(Error::external(format!("herdr: {}", combined(&output))));
    }
    Ok(())
}

/// Starts `cmd` without waiting for it; a background thread reaps it so it
/// never lingers as a zombie.
fn detach(cmd: &mut Command) -> std::io::Result<()> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The platform name used by [`desktop_command`].
fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

/// Opens a standalone terminal that does not use this process's stdio.
/// Commands are passed as argv; only macOS Terminal requires shell text.
fn open_window(ctx: &Ctx, terminal: &str, dir: &Path, argv: &[String]) -> Result<()> {
    let lookup =
        |name: &str| toolchain::lookup(ctx, name).map(|path| path.to_string_lossy().into_owned());
    let (program, args) =
        desktop_command(platform(), terminal, &dir.to_string_lossy(), argv, &lookup)?;
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .current_dir(dir)
        .env("PATH", toolchain::child_path(ctx));
    configure_desktop_process(&mut cmd, &program);
    detach(&mut cmd).map_err(|err| msg!("abrindo terminal: %w", Error::from(err)))
}

#[cfg(windows)]
fn configure_desktop_process(cmd: &mut Command, program: &str) {
    use std::os::windows::process::CommandExt;
    let name = base_name("windows", program).to_lowercase();
    if name == "powershell.exe" || name == "pwsh.exe" {
        // CREATE_NEW_CONSOLE: a windowless parent has no console to share.
        cmd.creation_flags(0x0000_0010);
    }
}

#[cfg(not(windows))]
fn configure_desktop_process(_cmd: &mut Command, _program: &str) {}

fn base_name<'a>(platform: &str, path: &'a str) -> &'a str {
    let separator = |c: char| c == '/' || (platform == "windows" && c == '\\');
    path.trim_end_matches(separator)
        .rsplit(separator)
        .next()
        .unwrap_or(path)
}

/// The program and arguments that open `argv` (or a shell when empty) in `dir`
/// in a new terminal window. `terminal` names the terminal program; when empty
/// the platform default is used. Pure so every platform is testable anywhere.
fn desktop_command(
    platform: &str,
    terminal: &str,
    dir: &str,
    argv: &[String],
    lookup: &dyn Fn(&str) -> Result<String>,
) -> Result<(String, Vec<String>)> {
    let strings = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let mut terminal = terminal.to_string();
    if terminal.is_empty() {
        if platform == "darwin" {
            let mut command = format!("cd -- {}", quote(&[dir.to_string()]));
            if !argv.is_empty() {
                command.push_str(" && ");
                command.push_str(&quote(argv));
            }
            let script = format!(
                "tell application \"Terminal\" to do script {}",
                script_string(&command)
            );
            let args = strings(&[
                "-e",
                &script,
                "-e",
                "tell application \"Terminal\" to activate",
            ]);
            return Ok(("/usr/bin/osascript".to_string(), args));
        }
        let candidates: &[&str] = if platform == "windows" {
            &["wt.exe", "pwsh.exe", "powershell.exe"]
        } else {
            &[
                "kitty",
                "foot",
                "wezterm",
                "gnome-terminal",
                "konsole",
                "xterm",
            ]
        };
        terminal = candidates
            .iter()
            .find_map(|candidate| lookup(candidate).ok())
            .unwrap_or_default();
    }
    if terminal.is_empty() {
        return Err(msg!(
            "nenhum terminal desktop encontrado; configure desktop_terminal"
        ));
    }
    let path = lookup(&terminal)?;
    let base = base_name(platform, &terminal);
    let args = match base.strip_suffix(".exe").unwrap_or(base) {
        "pwsh" | "powershell" => {
            let mut args = strings(&["-NoExit", "-NoLogo"]);
            if !argv.is_empty() {
                let quoted: Vec<String> = argv
                    .iter()
                    .map(|arg| format!("'{}'", arg.replace('\'', "''")))
                    .collect();
                args.push("-Command".to_string());
                args.push(format!("& {}", quoted.join(" ")));
            }
            args
        }
        "wt" => [strings(&["-d", dir]).as_slice(), argv].concat(),
        "wezterm" => with_command(strings(&["start", "--cwd", dir]), "--", argv),
        "gnome-terminal" => with_command(vec![format!("--working-directory={dir}")], "--", argv),
        _ => with_command(Vec::new(), "-e", argv),
    };
    Ok((path, args))
}

/// Appends `separator` and `argv` to a terminal's own options, or nothing when
/// there is no command and the terminal should start its default shell.
fn with_command(mut args: Vec<String>, separator: &str, argv: &[String]) -> Vec<String> {
    if !argv.is_empty() {
        args.push(separator.to_string());
        args.extend_from_slice(argv);
    }
    args
}

/// A double-quoted string literal with the escapes of Go's `strconv.Quote`,
/// which the AppleScript sent to Terminal has always been built with.
fn script_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{07}' => out.push_str("\\a"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0b}' => out.push_str("\\v"),
            c if c.is_control() && (c as u32) < 0x80 => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Joins argv into a POSIX shell command line.
pub fn quote(argv: &[String]) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c);
    let parts: Vec<String> = argv
        .iter()
        .map(|arg| {
            if !arg.is_empty() && arg.chars().all(safe) {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect();
    parts.join(" ")
}

/// Opens a URL with the platform opener.
pub fn browser(url: &str) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    } else {
        Command::new("xdg-open")
    };
    Ok(detach(cmd.arg(url))?)
}

/// Opens a directory in the platform file manager.
pub fn folder(dir: &Path) -> Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    Ok(detach(Command::new(opener).arg(dir))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::tests::env_lock;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    fn found(name: &str) -> Result<String> {
        Ok(name.to_string())
    }

    #[test]
    fn quote_joins_a_posix_command_line() {
        assert_eq!(
            quote(&strings(&["hunk", "diff", "a b", "it's", ""])),
            r"hunk diff 'a b' 'it'\''s' ''"
        );
        // Compared with Go's Quote for the same argv.
        assert_eq!(
            quote(&strings(&["a-_./=:@%+,Z9", "é", "a b", "", "x'y"])),
            r"a-_./=:@%+,Z9 'é' 'a b' '' 'x'\''y'"
        );
    }

    #[test]
    fn desktop_terminal_arguments() {
        let argv = strings(&["git", "diff", "a b;echo nope"]);
        for (terminal, want) in [
            ("kitty", strings(&["-e", "git", "diff", "a b;echo nope"])),
            (
                "wezterm",
                strings(&[
                    "start",
                    "--cwd",
                    "/repo with spaces",
                    "--",
                    "git",
                    "diff",
                    "a b;echo nope",
                ]),
            ),
            (
                "wt.exe",
                strings(&["-d", "/repo with spaces", "git", "diff", "a b;echo nope"]),
            ),
        ] {
            let (_, args) =
                desktop_command("linux", terminal, "/repo with spaces", &argv, &found).unwrap();
            assert_eq!(args, want, "{terminal}");
        }
        let (_, args) = desktop_command(
            "darwin",
            "",
            "/repo's folder",
            &strings(&["git", "diff"]),
            &found,
        )
        .unwrap();
        assert!(args[1].contains(r"'\\''"), "AppleScript quoting = {args:?}");
        let missing = |_: &str| Err(Error::external("missing"));
        assert!(
            desktop_command("linux", "", "/repo", &[], &missing).is_err(),
            "missing terminal accepted"
        );
    }

    #[test]
    fn powershell_preserves_arguments_as_literals() {
        let argv = strings(&["git", "diff", "a'; Remove-Item x"]);
        let (_, args) =
            desktop_command("windows", "powershell.exe", r"C:\repo", &argv, &found).unwrap();
        let want = [
            "-NoExit",
            "-NoLogo",
            "-Command",
            "& 'git' 'diff' 'a''; Remove-Item x'",
        ];
        assert_eq!(args, strings(&want));
    }

    /// The expected values were printed by Go's `desktopCommand` for the same
    /// inputs, with a lookup that prefixes "/bin/".
    #[test]
    fn desktop_command_matches_the_go_implementation() {
        let lookup = |name: &str| Ok(format!("/bin/{name}"));
        let argv = strings(&["git", "diff", "a b;echo \"x\"\t\\ é"]);
        let last = "a b;echo \"x\"\t\\ é";
        let activate = "tell application \"Terminal\" to activate";
        let cases = [
            (
                "darwin",
                "",
                "/usr/bin/osascript",
                strings(&[
                    "-e",
                    "tell application \"Terminal\" to do script \"cd -- '/repo'\\\\''s dir' && git diff 'a b;echo \\\"x\\\"\\t\\\\ é'\"",
                    "-e",
                    activate,
                ]),
                strings(&[
                    "-e",
                    "tell application \"Terminal\" to do script \"cd -- '/repo'\\\\''s dir'\"",
                    "-e",
                    activate,
                ]),
            ),
            (
                "linux",
                "",
                "/bin//bin/kitty",
                strings(&["-e", "git", "diff", last]),
                Vec::new(),
            ),
            (
                "windows",
                "",
                "/bin//bin/wt.exe",
                strings(&["-d", "/repo's dir", "git", "diff", last]),
                strings(&["-d", "/repo's dir"]),
            ),
            (
                "linux",
                "gnome-terminal",
                "/bin/gnome-terminal",
                strings(&["--working-directory=/repo's dir", "--", "git", "diff", last]),
                strings(&["--working-directory=/repo's dir"]),
            ),
            (
                "linux",
                "konsole",
                "/bin/konsole",
                strings(&["-e", "git", "diff", last]),
                Vec::new(),
            ),
            (
                "windows",
                "pwsh.exe",
                "/bin/pwsh.exe",
                strings(&[
                    "-NoExit",
                    "-NoLogo",
                    "-Command",
                    "& 'git' 'diff' 'a b;echo \"x\"\t\\ é'",
                ]),
                strings(&["-NoExit", "-NoLogo"]),
            ),
        ];
        for (platform, terminal, program, with_argv, without_argv) in cases {
            let got = desktop_command(platform, terminal, "/repo's dir", &argv, &lookup).unwrap();
            assert_eq!(
                got,
                (program.to_string(), with_argv),
                "{platform} {terminal:?}"
            );
            let got = desktop_command(platform, terminal, "/repo's dir", &[], &lookup).unwrap();
            assert_eq!(
                got,
                (program.to_string(), without_argv),
                "{platform} {terminal:?} shell"
            );
        }
    }

    #[test]
    fn windows_terminal_paths_select_their_arguments() {
        let (_, args) =
            desktop_command("windows", r"C:\Tools\wt.exe", r"C:\repo", &[], &found).unwrap();
        assert_eq!(args, strings(&["-d", r"C:\repo"]));
    }

    #[test]
    fn resolve_falls_back_to_a_window() {
        let _guard = env_lock();
        let saved = ["HERDR_ENV", "TMUX"].map(|key| (key, std::env::var_os(key)));
        // SAFETY: every test that reads or writes the environment holds env_lock.
        let set = |key: &str, value: &str| unsafe { std::env::set_var(key, value) };
        set("HERDR_ENV", "");
        set("TMUX", "");
        for setting in ["auto", "", "tmux", "inline"] {
            assert_eq!(resolve(setting), Mode::Window, "{setting:?} outside tmux");
        }
        set("TMUX", "/tmp/tmux-1000/default,1,0");
        assert_eq!(resolve("auto"), Mode::Tmux);
        assert_eq!(resolve("tmux"), Mode::Tmux);
        assert_eq!(resolve("inline"), Mode::Window);
        for (key, value) in saved {
            // SAFETY: as above.
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}
