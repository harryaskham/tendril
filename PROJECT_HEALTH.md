# Project health

## Handoff summary

Tendril is in a handoff-ready state for the `v0.0.4` slice.

Green surfaces:
- approved product contract lives in [`SPEC.md`](SPEC.md),
- human entrypoint lives in [`README.md`](README.md),
- published docs source and deployment live in [`docs/`](docs/), [`scripts/build-docs.sh`](scripts/build-docs.sh), and [`.github/workflows/pages.yaml`](.github/workflows/pages.yaml),
- managed project/build/test wiring lives in [`.cacophony/config.yaml`](.cacophony/config.yaml) and [`.cacophony/project.yaml`](.cacophony/project.yaml),
- local pre-merge validation lives in [`scripts/pre-merge.sh`](scripts/pre-merge.sh),
- tag-only release automation lives in [`.github/workflows/tag-release.yml`](.github/workflows/tag-release.yml) and builds raw Cargo binaries on self-hosted x86_64-linux, aarch64-linux, and aarch64-darwin runners using the shared `updatable-cli` asset contract, and
- repository-root installation works with `cargo install --locked --path .`.

## Surface map

| Facet | Status | Evidence |
| --- | --- | --- |
| CLI / agent workflow | Ready | `crates/tendril/src/cli.rs`, `crates/tendril/src/commands/mod.rs` |
| MCP stdio parity | Ready | `crates/tendril/tests/mcp_parity.rs`, `crates/tendril/tests/mcp_external_smoke.rs`, `scripts/mcp-stdio-smoke.sh`, `crates/mcp-cli/src/lib.rs` |
| Runtime config | Ready | `crates/tendril/src/config.rs`, `$TENDRIL_CONFIG_DIR/config.yaml` |
| Packaging / release | Ready | `.github/workflows/tag-release.yml`, `scripts/stage-release-artifacts.sh`, `crates/tendril/src/update.rs` (`updatable-cli`) |
| Managed build / test surfaces | Ready | `.cacophony/project.yaml`, `caco build run`, `caco test run` |
| Local lint / pre-merge gate | Ready | `scripts/pre-merge.sh` |
| Changelog / semver | Ready | `CHANGELOG.md`, workspace version `0.0.4`; `tendril version` is read-only |
| License declaration | Ready | `LICENSE`, workspace `license = "MIT"` |
| Audio capture | Ready on Linux + macOS; probe-only on unwired lanes | `crates/tendril/src/listen.rs` drives real WAV capture (Linux PipeWire `pw-record`→`parecord`, PulseAudio `parecord`; macOS `afrecord` plus ffmpeg/avfoundation system loopback). Windows/Android and non-WAV formats fall back to a structured `probe_only` response |
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

1. **Extend audio artifact capture to the remaining platform lanes**
   - Status: Linux (PipeWire/PulseAudio) and macOS (CoreAudio `afrecord`, plus ffmpeg/avfoundation system loopback) now emit real WAV artifacts via `crates/tendril/src/listen.rs`; unwired lanes and non-WAV formats return a structured `probe_only` response by design.
   - Why: Windows/Android capture lanes and non-WAV output formats are still unimplemented.
   - Action: wire recorders for the remaining platforms and add non-WAV format support, keeping the structured `probe_only` fallback for unsupported paths.

2. **Enforce coverage targets in automation**
   - Why: the spec defines line/branch coverage goals, but the current pre-merge and tag-release surfaces do not measure them.
   - Action: add `cargo llvm-cov` or equivalent to `flake.nix`, `scripts/pre-merge.sh`, and the tagged release gate.

## Operator-facing status

The repository is ready for handoff for the implemented `v0.0.4` scope. Build, test, lint, config, docs publication, repository-root Cargo installation, shared `updatable-cli` updates, and portable self-hosted release assets are connected and documented. Remaining work is explicit and bounded: Windows/Android audio capture lanes, non-WAV formats, and automated coverage enforcement.
