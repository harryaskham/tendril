# Changelog

All notable changes to Tendril are documented in this file.
Format based on [Keep a Changelog](https://keepachangelog.com/).

Tendril follows [Semantic Versioning](https://semver.org/). Release notes are cut from SemVer tags in the form `vX.Y.Z`, and the `Unreleased` section tracks changes on `main` until the next tag is created.

## [Unreleased]

### Added
- `PROJECT_HEALTH.md` handoff summary that links the spec, docs, validation, and release surfaces and captures explicit follow-ups.
- MIT `LICENSE` file and release-artifact packaging that now ships the license and project health summary alongside the changelog and README.
- A dedicated macOS operator-validation guide with copy-pasteable `nix run` examples for `list`, `capture`, `run`, and MCP stdio, plus permission-prompt expectations and self-containment troubleshooting.
- A published Pi/Cacophony-facing MCP integration contract that documents the `tendril mcp stdio` launch expectations, desktop-session and permission assumptions, stable tool names/arguments, and semver alignment with Tendril's MCP schemas.
- An external-client MCP smoke script and integration test that initialize Tendril over stdio, verify `tools/list` schema metadata, and call the `list` tool against the built binary contract.
- A Linux/X11 packaged-smoke script and operator guide for validating packaged `list`/`capture` flows, with optional real-input smoke coverage for `run`.

### Changed
- Windows 11 discovery, capture, and input no longer depend on spawning `powershell`; Tendril now uses embedded Win32 bindings for packaged-binary self-containment and covers the native flow with Windows-focused unit tests.
- README now links the approved spec, managed validation commands, runtime config location, docs publication surface, handoff health summary, and packaged macOS/Linux smoke-validation examples.
- Linux/X11 discovery, capture, and input now use an embedded X11/XRandR/XTest backend instead of `xrandr`, `xprop`, `xwininfo`, `import`, or `xdotool` helper tools.
- The runtime dependency audit now reflects the self-contained Linux/X11 path, the self-contained Windows path, and the remaining packaged-runtime follow-ups.
- Cargo package metadata now carries shared repository and homepage information for the workspace crates.
- Tag-triggered GitHub Actions release automation remains backed by the Nix flake and local pre-merge checks.
- Seeded the changelog and release-note flow so future releases can prepend human-readable summaries when a new `vX.Y.Z` tag is pushed.

## [0.0.1] - 2026-04-09

### Added
- Bootstrapped the Tendril Rust workspace at version `0.0.1`, including the `tendril` CLI crate and the in-repo reusable `mcp-cli` support crate.
- Added the initial agent-facing command surface: `tendril list`, `tendril capture`, `tendril run`, `tendril alias`, `tendril listen`, and `tendril mcp stdio`.
- Added structured JSON and MCP envelopes, typed command models, config loading from `~/.config/tendril/config.yaml`, and agent-oriented help for the list → capture → run workflow.
- Added target discovery, screenshot capture with resize/remapping metadata, target-scoped input execution with DSL support, and probe-first audio capability diagnostics.
- Added cross-platform adapter scaffolding for macOS, Linux, and Windows 11 with explicit capability, permission, and structured error reporting.
- Added Nix flake packaging, reproducible checks, Cacophony project bootstrap, git hooks, and the local `scripts/pre-merge.sh` validation gate.
- Added integration, CLI/MCP parity, and platform contract test coverage for the initial stateless desktop automation workflow.
- Added explicit SemVer and repository metadata wiring in Cargo manifests and release documentation.
- Added reproducible `.#releaseArtifact` packaging plus local release helper scripts for canonical binary archives, checksums, and manifest metadata.
