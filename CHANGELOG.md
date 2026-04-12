# Changelog

All notable changes to Tendril are documented in this file.
Format based on [Keep a Changelog](https://keepachangelog.com/).

Tendril follows [Semantic Versioning](https://semver.org/). Release notes are cut from SemVer tags in the form `vX.Y.Z`, and the `Unreleased` section tracks changes on `main` until the next tag is created.

## [Unreleased]

### Added
- `PROJECT_HEALTH.md` handoff summary that links the spec, docs, validation, and release surfaces and captures explicit follow-ups.
- MIT `LICENSE` file and release-artifact packaging that now ships the license and project health summary alongside the changelog and README.
- A dedicated macOS operator-validation guide with copy-pasteable `nix run` examples for `list`, `capture`, `run`, and MCP stdio, plus permission-prompt expectations and self-containment troubleshooting.

### Changed
- README now links the approved spec, managed validation commands, runtime config location, docs publication surface, handoff health summary, and macOS smoke-validation examples.
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
