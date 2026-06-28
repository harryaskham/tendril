# Session summary — `tendril capture --camera` single-frame video-device capture

## Goal

Complete the operator-requested camera support: make `tendril capture --camera <id>`
grab a still frame from a video device, mirroring window/display capture (and free over
`--remote`/`--wsl-tunnel`/`--android` via the existing invocation proxy). Follows the
discovery slice (bd-aed538) where cameras became listable.

## Bead(s)

- `bd-f5bf3a` — tendril capture --camera: single-frame video-device capture via ffmpeg (macOS)
- (predecessor: `bd-aed538` — camera discovery in `tendril list` + Camera permission)

## Before state

- Failing tests: none (335 lib). `tendril list` listed cameras but there was no way to
  capture from them; `--camera` did not exist.

## After state

- Failing tests: none. `cargo clippy -p tendril --all-targets -- -D warnings` clean;
  `cargo test -p tendril --lib` = 338 passed; `mcp_external_smoke` + `mcp_parity` pass.
- `tendril capture --camera <id>` (and the MCP `capture` tool's `camera` arg) grab one
  PNG frame via `ffmpeg -f avfoundation` on macOS; `-o` writes the file, JSON returns
  base64. ffmpeg-absent and non-macOS return clear structured errors.

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt.
- `camera.rs`: `avfoundation_capture_args` (pure, tested), `png_dimensions` (dependency-free
  IHDR parse, tested), `capture_camera_frame` (ffmpeg spawn; ffmpeg-missing -> clear
  install hint; macOS-gated). The camera-activating spawn is not exercised in tests (it
  turns on the webcam light); the ffmpeg single-frame->PNG mechanics were ground-verified
  out-of-band with `ffmpeg -f lavfi -i testsrc` (no camera).
- `model.rs`: `CameraCaptureOutput`. `cli.rs`: global `--camera`. `commands/mod.rs`:
  `TargetScope.camera`; capture dispatch routes to the camera branch (mutually exclusive
  with `--window`/`--display`) on both CLI and the MCP `capture` tool; output reuses the
  `-o`/JSON envelope handling.
- Tests: +5 (arg-builder, 2x png_dimensions, +existing). Schema fixtures updated:
  `mcp_external_smoke` capture/run/list_elements property lists gained `camera` (TargetScope
  is flattened into those tools).
- `docs/src/cli/index.md` + `docs/src/mcp.md` updated.
- Behavioural delta: NEW capture path that activates a camera when invoked. No change to
  window/display/screen capture.

## Operator-takeaway

`tendril capture --camera <id>` now works end-to-end in code (ffmpeg AVFoundation, already
in the env). The one thing not machine-verifiable is the live grab against a real webcam
(it activates the camera, so tests deliberately don't run it) -- the first real-device run
wants Harry's eyes, and if the exact ffmpeg avfoundation args need a per-device tweak
(framerate/pixfmt) that's a one-line follow-up. Native AVFoundation (drop the ffmpeg dep)
and Linux V4L2 / Windows MF capture remain follow-ups.
