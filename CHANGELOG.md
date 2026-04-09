# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Tag-triggered GitHub Actions release automation backed by the Nix flake and local pre-merge checks.

## [0.0.1] - 2026-04-09

### Added
- Initial Tendril Rust workspace scaffold with the `tendril` CLI and in-repo `mcp-cli` library crate.
- Nix flake packaging, reproducible checks, and the local `scripts/pre-merge.sh` validation gate.
- Tag-only GitHub Actions validation and release publishing that packages the Tendril binary and source archive from reproducible Nix builds.
