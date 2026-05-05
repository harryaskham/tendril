# Session summary — inactive Wayland backend fallback

## Goal

Fix a Tendril dogfooding papercut discovered while running `target/debug/tendril --json list` in this managed checkout: `hyprctl` was on PATH but not connected to Hyprland, and Tendril treated that as a fatal platform adapter failure instead of falling through to other Wayland discovery backends or the normal unsupported-backend diagnostic.

## Bead(s)

- `bd-38cd13` — Treat inactive Wayland compositor commands as unavailable during discovery

## Before state

- Failing tests: the dogfood command `cargo run -q -p tendril -- --json list` returned a `platform_adapter_failure` with `hyprland_instance_signature not set! (is hyprland running?)`.
- Relevant metrics: the command never reached the intended fallback/unsupported Wayland backend message even though this host is not a usable Hyprland session.
- Context: `run_optional_command` already classified several compositor-not-running outputs as backend unavailable, but missed Hyprland's `hyprland_instance_signature not set` wording.

## After state

- Failing tests: none in focused validation.
- Relevant metrics: dogfooding now returns a structured `unsupported_capability` diagnostic listing supported Wayland discovery backends instead of a raw hyprctl adapter failure; queued `cargo test -p tendril inactive_wayland_compositor_output_is_treated_as_backend_unavailable`, `cargo check -p tendril --tests`, and `cargo clippy -p tendril --tests -- -D warnings` all passed.
- Context: inactive compositor output classification is isolated in a helper and covered by a regression test.

## Diff summary

- Commits: `ed0636e`
- Files touched: `crates/tendril/src/discovery.rs`
- Tests: +1 unit test covering inactive Hyprland output, generic unable-to-connect output, and non-unavailable parse failures.
- Behavioural delta: an installed but inactive `hyprctl` no longer aborts Wayland discovery; it is treated as unavailable so Tendril can try the rest of the backend matrix.

## Operator-takeaway

Dogfooding found a sharp edge in the supported-backend matrix: tools on PATH are not necessarily usable in the current compositor session. Tendril now distinguishes that case and reports the higher-level backend availability diagnostic agents can act on.
