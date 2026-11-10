//! pr-tracker follows pull requests, merge requests and issues across GitHub,
//! GitLab and Gitea instances through the gh, glab and tea CLIs.

// A release build for Windows is a GUI program; it must not open a console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod ui;

/// A GUI program has no console of its own; subcommands print to the one of
/// the shell that started them.
#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    // SAFETY: AttachConsole takes no pointers and only fails harmlessly when
    // the parent has no console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|command| cli::handles(command)) {
        attach_parent_console();
        if let Err(err) = cli::run(&args) {
            eprintln!("erro: {err}");
            std::process::exit(1);
        }
        return;
    }
    ui::run();
}
