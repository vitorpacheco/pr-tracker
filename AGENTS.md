# Repository Guidelines

## Project Structure & Module Organization

This is a Cargo workspace with two crates. `crates/core` (`pr-tracker-core`) holds the domain code: `config` manages TOML settings, `provider` integrates with GitHub, GitLab, and Gitea through `gh`/`glab`/`tea` (Bitbucket is a placeholder), `gitops` owns clones and worktrees, `launch` opens terminals, diffs, folders, and browsers, `cache` stores SQLite snapshots, `i18n` resolves the language and translates messages (catalog in `crates/core/src/i18n/en.json`; Portuguese source strings are the keys), `theme` reads and exports palettes, `toolchain` locates and runs CLIs, `app` holds the session, actions, and diagnostics, and `error` defines `pr_tracker_core::Error`. `crates/pr-tracker` is the single binary: `main.rs` dispatches, `cli.rs` implements the subcommands that print to the terminal, and `ui/` implements the window with Rune and `rune-kit` (pinned git revision in `crates/pr-tracker/Cargo.toml`). `packaging/linux/` holds the `.desktop` entry and icon. Tests live beside the code in `#[cfg(test)]` modules. User-facing notes belong in `README.md` or `docs/` (see `docs/development.md`); build artifacts go to `dist/` and `target/` and remain untracked.

## Build, Test, and Development Commands

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.99.1 with clippy and rustfmt). Building on Linux needs the libxkbcommon and Wayland development packages (`libxkbcommon-dev libwayland-dev` on Debian/Ubuntu, `libxkbcommon wayland` on Arch).

- `make run` opens the window; use `make run ARGS=doctor` to exercise a subcommand and `make run ARGS=--demo` for fictional data with no accounts or side effects.
- `make build` creates `./pr-tracker` (release build) with version metadata from Git.
- `make test` runs `cargo test --workspace`.
- `make fmt` runs `cargo fmt --all`; `make lint` runs `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `make check` runs `lint` and `test`. Run it before submitting changes.
- `make vuln` runs `cargo-audit` against the dependencies.
- `make snapshot` renders the demo window to `dist/preview/window.png` without opening a window; pass `ARGS="--scene settings --theme light"` to vary it. `pr-tracker --list-scenes` prints the scenes, which are defined in `crates/pr-tracker/src/ui/mod.rs`.
- `make package` builds the host platform's archive in `dist/` (`pr-tracker_<version>_<os>_<arch>.tar.gz`, `.zip` on Windows); `make checksums` writes `dist/checksums.txt`. There is no cross-compilation: each platform is packaged on its own runner.
- `make install` installs into `$(PREFIX)/bin` (default `~/.local/bin`).
- CI lives in `.github/workflows/`: `ci.yml` (lint, tests on Linux/macOS/Windows, cargo-audit; native packages for six OS/architecture combinations only on nightly, release and manual runs), `nightly.yml` (pre-release `nightly` on every push to `main`), `release.yml` (tags `v*`), `vulncheck.yml` (weekly scan).

## Coding Style & Naming Conventions

Use idiomatic Rust formatted by `rustfmt`, and keep clippy clean with warnings denied; run `make fmt` after editing. Module names are short, lowercase nouns. Types and traits use `PascalCase`, functions and variables use `snake_case`, and tests are named after the behavior they check (`approve_asks_first_and_cancel_runs_nothing`). Keep provider-specific behavior behind the `Client` trait in `crates/core/src/provider`. Propagate errors as `pr_tracker_core::Error` with useful context instead of logging inside library code: build application messages with `msg!`, whose format string is the Portuguese catalog key, and wrap text owned by a CLI or the OS in `Error::external` so it is never translated. In `ui/`, `state.rs` holds the state and applies the results of background work, and the views are pure functions of that state. Comments should explain intent or constraints, not restate code.

## Testing Guidelines

Use Rust's built-in test harness and place tests in a `#[cfg(test)]` module beside the implementation. Prefer table-driven tests for mappings and state conversions and `tempfile` for filesystem scenarios. Tests that read or change environment variables must hold the shared lock (`i18n::tests::env_lock()` in `crates/core`), because tests run in parallel in one process. Tests that depend on an external CLI fake it with a script that records its invocations and are gated with `#[cfg(unix)]`, or skip when the real tool (such as `git`) is missing; they never call an authenticated CLI or the network. UI tests drive `rune::Runtime` headlessly on the demo data: find elements by key, press them, send key events, and assert on the state. When adding a translated string, add its Portuguese key to `en.json`; a test checks that format verbs match. There is no enforced coverage threshold; cover new behavior and regressions, then run `make check`.

## Commit & Pull Request Guidelines

Use concise, imperative subjects such as `Add GitLab pipeline status`, keeping each commit focused. Pull requests should explain the user-visible change, list verification performed, and link related issues. Include screenshots or recordings for UI changes (`make snapshot` renders them headlessly), and call out configuration, CLI compatibility, or worktree behavior changes explicitly.

## Security & Configuration

Never commit credentials or local configuration. Authentication is delegated to `gh`, `glab`, and `tea`; pr-tracker handles no tokens. Use temporary paths via `PR_TRACKER_CONFIG` and `PR_TRACKER_CACHE` in tests and examples.
