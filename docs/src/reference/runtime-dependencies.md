# Runtime dependency audit

This page inventories every current Tendril runtime subprocess/tool dependency in
`crates/tendril/src/discovery.rs`, `crates/tendril/src/elements.rs`,
`crates/tendril/src/platform.rs`, and `crates/tendril/src/wayland_input.rs`.

The governing rule from the approved `SPEC.md` is that every command should be
self-contained. This audit therefore distinguishes between:

- **documented platform prerequisite**: an OS-vended or environment-coupled tool
  that is acceptable to document for v0.0.1, even if a native embedding would be
  better later, and
- **self-containment/usability bug**: an external runtime dependency that makes a
  packaged Tendril binary materially less usable for operators.

`alias` and `mcp stdio` do not currently spawn extra platform helper
programs in the Tendril runtime path. `listen` shells out to a recorder
binary on platforms where actual capture is wired (see the `listen` rows
below); when no recorder is available it degrades to probe-only diagnostics
without spawning anything.

## Command and platform matrix

| CLI command | Platform/session | Current subprocess/tool dependency | Current use | Classification | Native/embedded direction | Tracking |
| --- | --- | --- | --- | --- | --- | --- |
| `list` | macOS | `osascript` | Quartz/AppKit discovery script via JXA for displays and windows | Documented platform prerequisite | Native Rust bindings would remove the subprocess boundary, but packaged usability no longer depends on the Swift toolchain | `bd-5c3937` |
| `list` | Linux/X11 | _none_ | Native X11/XRandR/EWMH discovery path | Self-contained today | Continue hardening the embedded backend and add more real-session smoke coverage | `bd-a279ed` |
| `list` | Linux/Wayland | `hyprctl` | Hyprland monitor/client discovery | Documented backend prerequisite today | Prefer compositor-native bindings or another embedded backend if a stable option exists | `bd-e4edee` |
| `list` | Linux/Wayland | `swaymsg` | sway output/tree discovery | Documented backend prerequisite today | Prefer compositor-native bindings or another embedded backend if a stable option exists | `bd-e4edee` |
| `list` | Linux/Wayland | `wlr-randr` | wlroots output enumeration fallback | Documented backend prerequisite today | Evaluate portal/native alternatives for supported matrices | `bd-e4edee` |
| `list` | Windows 11 | _none_ | Native Win32 monitor/window enumeration inside the Tendril binary | Self-contained | Keep the embedded Win32 backend covered by binary-flow tests | `bd-a3357b` |
| `capture` | macOS | `screencapture` | Window/display PNG capture | Documented platform prerequisite | Replace with native ScreenCaptureKit/Quartz capture when ready for tighter packaging control | audited in `bd-d513d4` |
| `capture` | Linux/X11 | _none_ | Native X11 image capture path | Self-contained today | Continue validating packaged flows against real X11 sessions | `bd-a279ed` |
| `capture` | Linux/Wayland | `grim` | Compatibility fallback when portal screenshot capture is unavailable | Documented compatibility fallback | Keep the portal-backed path primary; retain `grim` only as a clearly diagnosed fallback for supported compositor families | `bd-e4edee` |
| `capture` | Windows 11 | _none_ | Native Win32/GDI capture inside the Tendril binary | Self-contained | Continue hardening the embedded capture backend and smoke coverage | `bd-a3357b` |
| `run` | macOS | `osascript` | Focus transfer by PID/app name, text entry, key events, mouse clicks/drags via JXA/AppleScript | Documented platform prerequisite | Native accessibility/input bindings would still reduce subprocess overhead, but packaged usability no longer depends on the Swift toolchain | `bd-5c3937` |
| `run` | Linux/X11 | _none_ | Native X11/XTest focus, keyboard, and mouse injection | Self-contained today | Continue real-session validation, especially around keyboard-map edge cases | `bd-a279ed` |
| `run` | Linux/Wayland | `ydotool` | Preferred Wayland keyboard + pointer injection via uinput; requires the `ydotoold` daemon (`bd-408572`) | Documented platform prerequisite | Embed a uinput-based driver to remove the helper boundary once permissions and packaging are sorted | `bd-408572` |
| `run` | Linux/Wayland | `wtype` | Keyboard-only fallback via the wlroots `virtual-keyboard-v1` protocol when `ydotool` is not installed (`bd-408572`) | Documented platform prerequisite | Embed a wlroots virtual-keyboard client to remove the helper boundary | `bd-408572` |
| `run` | Linux/Wayland | _none_ | Generic Wayland input falls back to a structured `unsupported_capability` error when neither helper tool is installed | Not applicable | Keep the actionable diagnostic in sync with the helper-tool detection in `wayland_input::detect_backend` | `bd-408572` |
| `list-elements` | Linux/Wayland | _none_ | AT-SPI over D-Bus (`AT_SPI_BUS_ADDRESS` or `org.a11y.Bus.GetAddress`) for semantic element discovery; falls back to compositor target roots when unavailable | Self-contained with platform accessibility service | Keep the platform contract documented and validate against real Wayland applications that publish AT-SPI metadata | `bd-f22b95` |
| `run` | Windows 11 | _none_ | Native Win32 focus transfer plus keyboard/mouse injection inside the Tendril binary | Self-contained | Continue hardening the embedded input backend and smoke coverage | `bd-a3357b` |
| `listen` | macOS | `afrecord` | CoreAudio-backed WAV capture; ships with the OS | Documented platform prerequisite | Replace with a native CoreAudio binding once the cross-platform listen surface stabilizes | `bd-d7c2f0` |
| `listen` | Linux/PipeWire | `pw-record` | WAV capture against `@DEFAULT_MONITOR@` / `@DEFAULT_SOURCE@` with a sample-count limit so the recorder exits on its own | Documented backend prerequisite | Embed a PipeWire client to remove the subprocess boundary | `bd-d7c2f0` |
| `listen` | Linux/PulseAudio | `parecord` | WAV capture against `@DEFAULT_MONITOR@` / `@DEFAULT_SOURCE@`; also used as a fallback when `pw-record` is missing on PipeWire hosts | Documented backend prerequisite | Embed a PulseAudio/PipeWire client to remove the subprocess boundary | `bd-d7c2f0` |
| `listen` | * (recorder cleanup) | `kill` | Sent as `kill -TERM` to long-running recorders so they finalize the WAV header before exit (used only by the `parecord` path; `pw-record` exits on its own) | Documented platform prerequisite | Switch to in-process signal delivery once the workspace permits scoped `unsafe_code` for libc::kill | `bd-d7c2f0` |
| `listen` | Windows / unknown | _none_ | No real capture is wired; the JSON envelope reports `status = "probe_only"` with diagnostics | Not applicable | Wire WASAPI loopback/capture once a self-contained Rust path is selected | `bd-d7c2f0` |

## Classification summary

### Acceptable documented prerequisites for v0.0.1

These still deserve future native work, but they do not currently require most
operators to install extra package-manager tooling just to use the packaged
binary on a supported host:

- macOS `screencapture`
- macOS `osascript`
- Linux display server discovery probes `XDG_RUNTIME_DIR/wayland-*` and `/tmp/.X11-unix/X*` when `WAYLAND_DISPLAY`/`DISPLAY` are not already exported, so SSH and non-login shells can still identify the active local display server before compositor-specific discovery starts.
- Wayland compositor utilities (`hyprctl`, `swaymsg`, `wlr-randr`) when Tendril
  is explicitly operating against those compositor families
- Linux/Wayland `grim` only as an explicitly documented compatibility fallback
  after the preferred xdg-desktop-portal screenshot path has been tried
- Linux/Wayland `ydotool` (with the `ydotoold` daemon) as the preferred input
  backend, plus `wtype` as the keyboard-only fallback for wlroots compositors
  (`bd-408572`)
- Linux/Wayland AT-SPI accessibility service for semantic `list-elements`
  metadata; it is a desktop-session service accessed over D-Bus, not an extra
  Tendril subprocess (`bd-f22b95`)

Why they are only *conditionally* acceptable:

- they are still subprocess boundaries rather than embedded implementations,
- they can still degrade startup latency and diagnostics, and
- they still need operator-facing docs so packaged-binary users know what
  environment Tendril expects.

### Self-containment/usability bugs still open

These dependencies materially undermine the “download one binary and run it”
goal. Windows PowerShell-backed discovery, capture, and input were removed in
`bd-a3357b`, so they no longer appear in this section:

- Linux/Wayland capture backend availability remains a packaging/documentation
  concern because operators still need either a working xdg-desktop-portal
  screenshot backend or the documented `grim` fallback for some sessions
  (`bd-e4edee`)

## Recent self-containment improvements

Two major packaged-binary blockers have already been removed in the current
source tree:

- macOS no longer depends on a runtime Swift toolchain for discovery and input
  (`bd-5c3937`)
- Linux/X11 no longer depends on `xrandr`, `xprop`, `xwininfo`, `import`, or
  `xdotool`; the packaged flow now uses an embedded X11 backend plus operator
  smoke coverage (`bd-a279ed`)

## Prioritized native/embedded follow-up work

1. **Wayland capture/backend hardening** — `bd-e4edee` now treats compositor-
   coupled discovery as a documented matrix, prefers xdg-desktop-portal for
   capture, and retains `grim` only as a compatibility fallback with clearer
   diagnostics for missing session tools.
2. **Further macOS/Linux backend hardening** — the major helper-tool blockers
   are removed, but more real-session packaged smoke coverage and edge-case
   polish remain worthwhile.

## Operator-facing validation

For packaged-binary validation on real hosts, use the dedicated guides:

- [macOS operator validation](../macos-operator-validation.md)
- [Linux/X11 operator validation](../linux-x11-operator-validation.md)
- [Linux Wayland operator validation](../linux-wayland-operator-validation.md)
