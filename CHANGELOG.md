# Changelog

All notable changes to Tendril are documented in this file.
Format based on [Keep a Changelog](https://keepachangelog.com/).

Tendril follows [Semantic Versioning](https://semver.org/). Release notes are cut from SemVer tags in the form `vX.Y.Z`, and the `Unreleased` section tracks changes on `main` until the next tag is created.

## [Unreleased]

### Added
- Tag-triggered GitHub Actions release automation backed by the Nix flake and local pre-merge checks.

### Changed
- Seeded the changelog and release-note flow so future releases can prepend human-readable summaries when a new `vX.Y.Z` tag is pushed.

## [v0.0.1] - 2026-04-09

### Added
- Bootstrapped the Tendril Rust workspace at version `0.0.1`, including the `tendril` CLI crate and the in-repo reusable `mcp-cli` support crate.
- Added the initial agent-facing command surface: `tendril list`, `tendril capture`, `tendril run`, `tendril alias`, `tendril listen`, and `tendril mcp stdio`.
- Added structured JSON and MCP envelopes, typed command models, config loading from `~/.config/tendril/config.yaml`, and agent-oriented help for the list → capture → run workflow.
- Added target discovery, screenshot capture with resize/remapping metadata, target-scoped input execution with DSL support, and probe-first audio capability diagnostics.
- Added cross-platform adapter scaffolding for macOS, Linux, and Windows 11 with explicit capability, permission, and structured error reporting.
- Added Nix flake packaging, reproducible checks, Cacophony project bootstrap, git hooks, and the local `scripts/pre-merge.sh` validation gate.
- Added integration, CLI/MCP parity, and platform contract test coverage for the initial stateless desktop automation workflow.
