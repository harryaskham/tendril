# Project health

## Handoff summary

Tendril is in a handoff-ready state for the approved `v0.0.1` slice.

Green surfaces:
- approved product contract lives in [`SPEC.md`](SPEC.md),
- human entrypoint lives in [`README.md`](README.md),
- published docs source and deployment live in [`docs/`](docs/), [`scripts/build-docs.sh`](scripts/build-docs.sh), and [`.github/workflows/pages.yaml`](.github/workflows/pages.yaml),
- managed project/build/test wiring lives in [`.cacophony/config.yaml`](.cacophony/config.yaml) and [`.cacophony/project.yaml`](.cacophony/project.yaml),
- local pre-merge validation lives in [`scripts/pre-merge.sh`](scripts/pre-merge.sh),
- tagged release automation lives in [`.github/workflows/tag-release.yml`](.github/workflows/tag-release.yml), [`scripts/release-artifacts.sh`](scripts/release-artifacts.sh), and [`scripts/release-notes.sh`](scripts/release-notes.sh), and
- release packaging now carries [`README.md`](README.md), [`CHANGELOG.md`](CHANGELOG.md), [`LICENSE`](LICENSE), and this summary for operator handoff context.

## Surface map

| Facet | Status | Evidence |
| --- | --- | --- |
| CLI / agent workflow | Ready | `crates/tendril/src/cli.rs`, `crates/tendril/src/commands/mod.rs` |
| MCP stdio parity | Ready | `crates/tendril/tests/mcp_parity.rs`, `crates/mcp-cli/src/lib.rs` |
| Runtime config | Ready | `crates/tendril/src/config.rs`, `$TENDRIL_CONFIG_DIR/config.yaml` |
| Packaging / release | Ready | `flake.nix`, `.github/workflows/tag-release.yml`, `scripts/release-artifacts.sh` |
| Managed build / test surfaces | Ready | `.cacophony/project.yaml`, `caco build run`, `caco test run` |
| Local lint / pre-merge gate | Ready | `scripts/pre-merge.sh` |
| Changelog / semver | Ready | `CHANGELOG.md`, workspace version `0.0.1` |
| License declaration | Ready | `LICENSE`, workspace `license = "MIT"` |
| Audio capture | Partial by design | `tendril listen` is probe-first and explicitly documents artifact emission as not yet implemented |
| Documentation publishing | Ready | `docs/book.toml`, `scripts/build-docs.sh`, `.github/workflows/pages.yaml` |
| Coverage enforcement | Follow-up | SPEC targets are documented, but no coverage gate is enforced in `flake.nix` or `scripts/pre-merge.sh` yet |

## Validation commands

Preferred operator/agent surfaces:

```bash
caco build run --wait true
caco test run --wait true
```

Local developer parity checks:

```bash
./scripts/pre-merge.sh
nix develop --command cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Known follow-ups

1. **Finish audio artifact capture beyond capability probing**
   - Why: `tendril listen` currently returns structured capability/permission diagnostics but not an emitted audio artifact.
   - Action: implement adapter-backed recording for the supported platform lanes and keep the current structured capability errors for unsupported paths.

2. **Enforce coverage targets in automation**
   - Why: the spec defines line/branch coverage goals, but the current pre-merge and tag-release surfaces do not measure them.
   - Action: add `cargo llvm-cov` or equivalent to `flake.nix`, `scripts/pre-merge.sh`, and the tagged release gate.

## Operator-facing status

The repository is ready for handoff for the implemented `v0.0.1` scope. Build, test, lint, config, packaging, docs publication, and release surfaces are connected and documented. The remaining work is explicit and bounded: full audio capture and automated coverage enforcement.
