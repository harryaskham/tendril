# Session summary — WSL Windows host tunnel

## Goal

Complete `bd-9ec67b` by adding a WSL tunnel mode that lets Tendril invoked from WSL/Linux proxy the same command to a Windows-host `tendril.exe`, while preserving Tendril JSON/MCP envelopes and composing with `--remote`.

## Bead(s)

- `bd-9ec67b` — Implement WSL tunnel mode for Tendril Windows host control

## Before state

- Failing tests: none known at session start.
- Relevant metrics: `--remote` could proxy to another Linux/macOS Tendril process, but there was no WSL-specific path for targeting the Windows host from inside WSL.
- Context: `bd-33b65c` landed native Windows list/capture/run/list-elements surfaces first, providing the Windows-host target binary the tunnel can invoke.

## After state

- Failing tests: none in focused validation.
- Relevant metrics: queued `cargo test -p tendril wsl` passed 2/2; queued `cargo test -p tendril cli::tests` passed 3/3; queued `cargo check -p tendril --tests` passed; queued `cargo clippy -p tendril --tests -- -D warnings` passed; `git diff --check` passed.
- Context: dogfooding `cargo run -q -p tendril -- --wsl-tunnel --json list` on this Linux host (without `tendril.exe`) returned a structured `wsl_tunnel_windows_binary_spawn_failed` error with setup guidance.

## Diff summary

- Commits: `471785f`
- Files touched: `crates/tendril/src/wsl.rs`, `crates/tendril/src/cli.rs`, `crates/tendril/src/lib.rs`, `crates/tendril/src/commands/mod.rs`, `README.md`, `docs/src/reference/platform-support.md`
- Tests: +1 WSL argument-stripping unit test and +1 CLI parse test.
- Behavioural delta: `--wsl-tunnel` is a global proxy flag. Without `--remote`, local Tendril strips only `--wsl-tunnel` and runs `tendril.exe` (or `TENDRIL_WSL_WINDOWS_BIN`) with the remaining arguments. With `--remote`, the flag is forwarded so the remote Tendril process can perform the Windows-host hop from inside WSL. MCP stdio uses streaming process I/O; normal JSON commands use captured I/O and structured setup/failure errors.

## Operator-takeaway

WSL callers now have a transparent tunnel to the Windows-host Tendril binary. The first version locates a visible `tendril.exe` or a `TENDRIL_WSL_WINDOWS_BIN` override and reports clear setup errors when the Windows binary is missing.
