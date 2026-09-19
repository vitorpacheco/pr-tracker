# Repository Guidelines

## Project Structure & Module Organization

`main.go` contains the CLI entry point and top-level commands. Keep domain code under `internal/`: `config` manages TOML settings, `provider` integrates with GitHub and GitLab through `gh`/`glab`, `gitops` owns clones and worktrees, `launch` opens terminals, diffs, and browsers, and `ui` implements the Bubble Tea interface. Tests live beside their packages as `*_test.go`. User-facing notes belong in `README.md` or `docs/`; build artifacts go to `dist/` and remain untracked.

## Build, Test, and Development Commands

- `make run` starts the terminal UI; use `make run ARGS=doctor` to exercise a subcommand.
- `make build` creates `./pr-tracker` with version metadata from Git.
- `make test` runs `go test ./...` across all packages.
- `make cover` writes `coverage.out` and prints total coverage.
- `make check` verifies formatting, runs `go vet`, and executes the test suite. Run it before submitting changes.
- `make vuln` runs `govulncheck` against the dependencies.
- `make dist` cross-compiles release binaries into `dist/`; `make package` also builds the archives and `checksums.txt` published by the release workflows.
- CI lives in `.github/workflows/`: `ci.yml` (lint, tests on Linux/macOS/Windows, build, govulncheck), `nightly.yml` (pre-release `nightly` on every push to `main`), `release.yml` (tags `v*`), `vulncheck.yml` (weekly scan).

## Coding Style & Naming Conventions

Use idiomatic Go and tabs as produced by `gofmt`; run `make fmt` after editing. Package names are short, lowercase nouns. Exported identifiers use `PascalCase`, private identifiers use `camelCase`, and tests follow `TestBehavior` naming. Keep provider-specific behavior behind the interfaces in `internal/provider`, and propagate errors with useful context instead of logging inside library packages. Comments should explain intent or constraints, not restate code.

## Testing Guidelines

Use Go's `testing` package and place tests next to the implementation. Prefer table-driven tests for mappings and state conversions, `t.TempDir()` for filesystem scenarios, and `t.Setenv()` for isolated configuration. External CLI-dependent tests should skip when the tool is unavailable. There is no enforced coverage threshold; cover new behavior and regressions, then run `make check`.

## Commit & Pull Request Guidelines

This repository has no commit history yet. Use concise, imperative subjects such as `Add GitLab pipeline status`, keeping each commit focused. Pull requests should explain the user-visible change, list verification performed, and link related issues. Include terminal screenshots or recordings for UI changes, and call out configuration, CLI compatibility, or worktree behavior changes explicitly.

## Security & Configuration

Never commit credentials or local configuration. Authentication is delegated to `gh` and `glab`; use temporary config paths via `PR_TRACKER_CONFIG` in tests and examples.
