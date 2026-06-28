# Session summary — macOS system-audio loopback capture via virtual device (ffmpeg)

## Goal

Resolve the auto-filed feedback bug that `tendril listen --source system` hard-fails on
macOS with "the macOS adapter spine does not expose system loopback capture." macOS has no
built-in system audio loopback, but a virtual loopback device (BlackHole) can provide it.
Make `--source system` capture from a detected virtual loopback device instead of erroring.

## Bead(s)

- `bd-d92c7e` — unsupported capability: macOS system loopback capture (auto-filed feedback/omni)

## Before state

- Failing tests: none. `tendril listen --source system` on macOS returned a hard
  `unsupported_capability` error at the probe stage (before any recorder ran), which the
  feedback pipeline auto-filed. Separately, the only wired macOS recorder was `afrecord`,
  which is NOT present on this host — so macOS audio capture was effectively probe-only.

## After state

- Failing tests: none. clippy --all-targets clean; `cargo test -p tendril --lib` = 344 passed;
  `mcp_external_smoke` + `mcp_parity` pass.
- macOS gains an `ffmpeg` AVFoundation recorder (afrecord kept as fallback). `--source system`
  detects a virtual loopback device (BlackHole/Loopback/Aggregate/etc.) and captures from it;
  the probe returns supported with a note naming the device + routing reminder. With no
  virtual device present, it returns a clear, actionable unsupported error (install BlackHole
  + route system output).

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt.
- `listen.rs`: `parse_avfoundation_audio_devices` + `find_loopback_device` (tested),
  `detect_macos_loopback_device` (ffmpeg, macOS-gated), `ffmpeg_avfoundation_audio_args`
  (tested) + `build_ffmpeg_avfoundation_args`; macOS `recorders_for` now [ffmpeg, afrecord].
- `platform.rs`: `MacOsAdapter::system_loopback_capability` (pure decision fn, both branches
  tested) wired into the probe via `detect_macos_loopback_device()`.
- `docs/src/cli/listen.md` updated.
- Tests: +5 (audio-device parse, loopback match x2, ffmpeg audio args, probe decision).
  Updated 2 tests (recorder selection now ffmpeg-first; probe test now deterministic).
- Behavioural delta: `--source system` on macOS captures from a virtual loopback device
  instead of hard-erroring; this also fixes macOS audio capture being probe-only (afrecord absent).

## Operator-takeaway

`tendril listen --source system` now works on macOS for anyone with BlackHole (or similar)
routing system output — the dead-end "unsupported in v0.0.1" error is gone, replaced by real
capture or actionable setup guidance. The ffmpeg recorder also fixes macOS audio capture being
silently probe-only because the legacy `afrecord` isn't installed. The actual captured audio
(does BlackHole have routed system sound) is the operator-only verification step; ground
mechanics (ffmpeg audio->WAV) were verified via `ffmpeg -f lavfi -i sine`.
