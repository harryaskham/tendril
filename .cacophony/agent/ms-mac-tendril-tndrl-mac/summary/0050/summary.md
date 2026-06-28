# Session summary — camera/video-device discovery in `tendril list` (+ Camera permission)

## Goal

Begin the operator-requested non-display video-device (camera) support. Harry asked
whether `tendril capture --camera` could work "basically the same" as window/display
capture, including `--remote` and listing in `tendril list`. This slice lands the
foundation: cameras are discoverable via `tendril list`, and the permission surface
reports Camera consent. The camera-activating single-frame capture path is the
follow-up (it needs live verification against a real webcam, which can't be done
headless without popping the operator's camera).

## Bead(s)

- `bd-aed538` — Camera/video-device capture: `tendril list` enumeration + Camera permission (ffmpeg-v1 plan, macOS)

## Before state

- Failing tests: none (335 lib... was 328). Note: `mcp_parity` was silently broken on
  `origin/main` — its `tool_names` list was missing `permissions` (introduced by
  bd-39d596; the reintegration gate runs `test-small`, not the heavier MCP integration
  tests, so it slipped through). This session surfaced and fixed it.
- `TargetKind`/`CaptureTargetKind` had only Window/Display; no camera concept anywhere.
- `tendril list` returned only windows/displays; `permissions` covered Screen Recording,
  Accessibility, Microphone.

## After state

- Failing tests: none. `cargo clippy -p tendril --all-targets -- -D warnings` clean;
  `cargo test -p tendril --lib` = 335 passed; `mcp_external_smoke` + `mcp_parity` pass.
- `tendril list` now includes a `cameras[]` array (macOS, via built-in
  `system_profiler SPCameraDataType -json`); `--remote`/`--wsl-tunnel`/`--android` get it
  free via the existing invocation proxy.
- `permissions` (CLI + MCP) now reports a Camera row on every adapter.

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt.
- New `crates/tendril/src/camera.rs`: `parse_spcamera_json` (tested) + `enumerate_cameras`
  (macOS system_profiler; empty elsewhere). Cameras are modeled as a SEPARATE device
  class, NOT threaded through `CaptureTargetKind` (120 usages across input/elements/x11/
  wayland where "camera" is nonsensical — cameras can be captured but never receive input
  or expose UI elements).
- `model.rs`: `CameraDescriptor`, `ListOutput.cameras` (serde default + skip-if-empty for
  backward compat), `ListInput.include_cameras` (default true).
- `platform.rs`: `PermissionKind::Camera`; macOS `camera_permission()`; `PlatformAdapter::cameras()`
  default-empty trait method overridden by `MacOsAdapter`; Linux/Windows `not_required` Camera rows.
- `commands/mod.rs`: list assembly populates `cameras`; human renderer prints a cameras
  section; `permission_kind_label` Camera arm.
- `android.rs`: ListOutput construction updated. `tests/mcp_parity.rs`: added the missing
  `permissions` tool (broken-on-main fix). `docs/src/cli/index.md` updated.
- Tests: +8 (4 system_profiler parser, 3 model serde/default, 1 permission coverage).
- Behavioural delta: discovery-only; no capture path yet, so nothing activates a camera.

## Operator-takeaway

`tendril list` can now SEE cameras and `permissions` reports Camera consent, on a clean
separate-device-class model that avoids polluting the window/display input/element
machinery. The actual `tendril capture --camera` frame grab (ffmpeg `-f avfoundation`,
already in the env) is the deliberate follow-up because its core effect activates the
webcam and needs live verification — best done with Harry present. Also fixed a latent
`mcp_parity` broken-on-main (missing `permissions` tool entry) that the test-small gate
had been skipping.
