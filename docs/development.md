# Development

pr-tracker is one Rust binary. With no arguments it opens a native window drawn
with [Rune](https://github.com/anomalyco/rune), a GPU-rendered Rust UI
framework, and its `rune-kit` components. With a subcommand it prints to the
terminal (`crates/pr-tracker/src/cli.rs`). Both share the configuration,
providers, and SQLite cache in `crates/core`.

## Requirements

- Rust 1.99.1, pinned in `rust-toolchain.toml` together with clippy and
  rustfmt. `rustup` installs it on first use; `mise.toml` pins the same version.
- Rune and `rune-kit` are git dependencies pinned to one revision in
  `crates/pr-tracker/Cargo.toml`. Cargo fetches them; nothing else is needed.
- Linux: the libxkbcommon and Wayland development packages to build, and a
  Vulkan driver to draw.

  ```sh
  sudo apt install libxkbcommon-dev libwayland-dev   # Debian/Ubuntu
  sudo pacman -S libxkbcommon wayland                # Arch
  ```

- `git`, `gh`, `glab`, and `tea` only matter at run time, for real data. Tests
  never call an authenticated CLI.

## Running

```sh
make run                 # the window, on your real configuration
make run ARGS=--demo     # the window on fictional data
make run ARGS=doctor     # a subcommand
make build               # release build, copied to ./pr-tracker
make install             # into ~/.local/bin (override with PREFIX=...)
```

`pr-tracker` uses your real configuration and cache. On first launch it creates
the configuration file and adds an instance for each host the installed CLIs
are already logged in to. Authentication stays with `gh`, `glab`, and `tea`;
pr-tracker never asks for tokens.

To try the real code paths without touching personal files, point both paths
somewhere disposable:

```sh
mkdir -p .cache/smoke
PR_TRACKER_CONFIG="$PWD/.cache/smoke/config.toml" \
PR_TRACKER_CACHE="$PWD/.cache/smoke/cache.db" \
cargo run -p pr-tracker
```

Window preferences (`window.json`) are stored beside the configuration file, so
they follow `PR_TRACKER_CONFIG`.

## Demo mode

`pr-tracker --demo` opens the window on the fictional data in
`crates/pr-tracker/src/ui/demo.rs`: two instances, a handful of PRs and issues,
checks, and a conversation. It reads no configuration, cache, or system
palette, starts no CLI, and saves nothing (neither settings nor window
preferences). A banner says so, and actions that would change something end
with a "Demo" error instead.

## Headless snapshots and scenes

Rune can render a frame to a PNG without opening a window. A GPU adapter is
still needed.

```sh
pr-tracker --demo --snap out.png
pr-tracker --demo --snap out.png --scene settings --theme light --size 1000x700 --scale 2
pr-tracker --list-scenes
make snapshot                          # dist/preview/window.png
make snapshot ARGS="--scene palette"
```

- `--scene NAME` applies a named state before drawing. The scenes are defined
  in `spec()` in `crates/pr-tracker/src/ui/mod.rs`: `issues`, `conversation`,
  `checks`, `settings`, `palette`, `confirm-merge`, `confirm-worktree`, `more`.
  To add one, add a `(name, |app| …)` entry there.
- `--theme dark|light` sets the system appearance the frame is drawn for
  (default dark).
- `--size WxH` sets the window size in logical pixels (default 1280x820) and
  `--scale N` the pixel scale.

Always pass `--demo` with `--snap`: without it the frame is drawn from your real
configuration and cache.

## Architecture of `crates/pr-tracker/src/ui`

| File | Role |
|---|---|
| `mod.rs` | The `AppSpec`: window title and sizes, `init`/`setup`/`poll`/`close`, keyboard shortcuts (`on_key`), palette commands, scenes, and the headless UI tests |
| `state.rs` | `App`: everything the window knows and every way it changes. Nothing in it draws |
| `view.rs` | Toolbar, sidebar, and the list |
| `inspector.rs` | The selected item: Details, Checks, Conversation, composer, presets, Approve/Merge, and the More actions menu |
| `dialogs.rs` | Confirmation, command palette, and settings |
| `settings.rs` | The editable copy of the configuration behind the settings dialog |
| `list.rs` | Filtering, grouping by repository, and selection movement |
| `theme.rs` | Turns a palette, or the built-in light and dark schemes, into the colors the views use |
| `prefs.rs` | `window.json`: theme preference, pane widths and collapsed state, comment drafts |
| `demo.rs` | Fictional data for `--demo` |
| `text.rs`, `widgets.rs` | Wording and small pieces shared by the views |

The rules that keep this simple:

- **State lives in `state.rs`.** Event handlers in the views call methods on
  `App` (`select`, `ask`, `execute`, `refresh`, `save_settings`, …). The views
  are pure functions of that state and hold none of their own.
- **Background work runs on threads.** `App::spawn` runs a closure against the
  shared `pr_tracker_core::app::Session` on a new thread; the closure returns an
  `Event` (`Loaded`, `Refreshed`, `Detail`, `Executed`, `SettingsSaved`,
  `Diagnosed`, `Palette`, `ThemeExported`), which is sent over a channel, and
  the thread then wakes the window. Nothing blocks the UI thread.
- **`poll` applies the results.** Rune calls `poll` on each wake; `App::tick`
  drains the channel through `apply`, starts the periodic refresh when it is
  due, and every two seconds re-reads the palette and saves changed window
  preferences. `next_wake` tells Rune when to call again.
- **Actions ask first.** `App::ask` opens the confirmation dialog for an action
  on the selected item; only `App::execute` runs it.
- **Core stays UI-free.** `Session` in `crates/core/src/app` owns the
  configuration generations, the refresh in progress, per-item actions, and
  cancellation. The window passes item keys, never arbitrary repositories.

The window opens at 1280 × 820 and cannot be made smaller than 760 × 480
(`SIZE` and `MIN_SIZE` in `mod.rs`).

## Adding a translated string

Interface messages are written in Portuguese in the code; the Portuguese text is
the catalog key.

1. Use the string through `app.tr("…")` in a view, or `msg!("…", args)` for an
   error in `crates/core`.
2. Add the key and its English text to `crates/core/src/i18n/en.json`.
3. Keep format verbs (`%s`, `%d`, `%q`, `%v`, `%w`) identical on both sides. The
   test `catalog_preserves_formatting` in `crates/core/src/i18n.rs` fails when
   they differ or a translation is empty.

A key missing from `en.json` is shown in Portuguese. Never pass repository
content (titles, comments, CLI output) through a translation call; wrap external
text in `Error::external`.

## Tests

```sh
make check    # cargo fmt --check, clippy -D warnings, cargo test --workspace
make test
make vuln     # cargo-audit
```

- Tests live in `#[cfg(test)]` modules beside the code and use `tempfile` for
  filesystem scenarios.
- Tests that read or change environment variables hold the shared lock,
  `i18n::tests::env_lock()` in `crates/core`, since tests run in parallel.
- Tests that need a provider CLI (see `provider/github.rs`) replace it with a
  shell script that records its invocations and replays canned responses; they
  are gated with `#[cfg(unix)]`.
  `gitops` tests use the real `git` in temporary repositories and skip when it
  is missing. No test approves a PR, posts a comment, or uses the network.
- UI tests in `ui/mod.rs` drive `rune::Runtime` headlessly on the demo data:
  they find elements by key, press them, send key events, step frames, and
  assert on `App`. Tests in `ui/state.rs` exercise `App` directly, without
  drawing.

## Packaging and CI

`make package` builds the release binary and archives it for the host platform
as `dist/pr-tracker_<version>_<os>_<arch>.tar.gz` (`.zip` on Windows), with
`README.md` and, on Linux, the contents of `packaging/linux/`. There is no
cross-compilation: the window links native graphics libraries, so `ci.yml`
packages each of the six OS/architecture combinations on its own runner.
`nightly.yml` and `release.yml` call `ci.yml`, collect the six archives, run
`make checksums`, and publish them.

`packaging/linux/` holds the `.desktop` entry and SVG icon; the application uses
the same app ID (`io.github.vitorpacheco.pr_tracker`). Neither is installed
automatically.

## Platform notes

- **Linux** is the platform the window has been run and verified on.
- **macOS and Windows** are built, tested, and packaged by CI only; the window
  has not been verified by hand there. macOS archives are not signed or
  notarized.
- On Windows a release build is a GUI program without a console of its own;
  subcommands attach to the console of the shell that started them.
