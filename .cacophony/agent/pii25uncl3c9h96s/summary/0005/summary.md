# Session summary — Linux display socket discovery

## Goal

Complete `bd-55de2e` by making Linux session detection infer X11 or Wayland from well-known display sockets when `DISPLAY`, `WAYLAND_DISPLAY`, or `XDG_SESSION_TYPE` are missing, which is common in SSH/non-login shells.

## Bead(s)

- `bd-55de2e` — Implement display server discovery for Linux (X11/Wayland)

## Before state

- Failing tests: none known at session start.
- Relevant metrics: Linux session detection only honored `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, and `DISPLAY`. Socket discovery existed in the remote shell bootstrap, but the local adapter context itself returned `Unknown` when those env vars were absent.
- Context: discovery backends for X11, Hyprland, sway, and wlroots were already implemented; the remaining gap was automatic session identification without pre-set display env vars.

## After state

- Failing tests: none in focused validation.
- Relevant metrics: queued `cargo test -p tendril linux_session_detection` passed 3/3; queued `cargo check -p tendril --tests` passed; queued `cargo clippy -p tendril --tests -- -D warnings` passed; `git diff --check` passed.
- Context: `AdapterContext::detect()` now passes `XDG_RUNTIME_DIR` and a filesystem probe into Linux session detection. The detector prefers explicit env/session type, then probes `XDG_RUNTIME_DIR/wayland-0..9`, then `/tmp/.X11-unix/X0..9`.

## Diff summary

- Commits: `9eee55c`
- Files touched: `crates/tendril/src/platform.rs`, `docs/src/reference/runtime-dependencies.md`
- Tests: +2 unit tests for Wayland and X11 socket inference without display env vars.
- Behavioural delta: Tendril can identify Linux Wayland or X11 sessions from standard sockets even when shell environment variables were not exported, allowing later target discovery to run against the correct backend.

## Operator-takeaway

The heavy compositor/X11 discovery was already present; this patch closes the lower-level detection gap so remote or non-login shells can pick the right Linux display backend from sockets before trying compositor-specific commands.
