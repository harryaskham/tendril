# Runtime dependency audit

This page inventories every current Tendril runtime subprocess/tool dependency in
`crates/tendril/src/discovery.rs` and `crates/tendril/src/platform.rs`.

The governing rule from the approved `SPEC.md` is that every command should be
self-contained. This audit therefore distinguishes between:

- **documented platform prerequisite**: an OS-vended/runtime-environment tool
  that is acceptable to document for v0.0.1, even if a native embedding would be
  better later, and
- **self-containment/usability bug**: an external runtime dependency that makes a
  packaged Tendril binary materially less usable for operators.

`listen`, `alias`, and `mcp stdio` do not currently spawn extra platform helper
programs in the Tendril runtime path.

## Command and platform matrix

| CLI command | Platform/session | Current subprocess/tool dependency | Current use | Classification | Native/embedded direction | Tracking |
| --- | --- | --- | --- | --- | --- | --- |
| `list` | macOS | `osascript` | Quartz/AppKit discovery script via JXA for displays and windows | Documented platform prerequisite | Native Rust bindings would remove the subprocess boundary, but packaged usability no longer depends on the Swift toolchain | `bd-5c3937` |
| `list` | Linux/X11 | `xrandr` | Display enumeration | **Bug** | Replace with native X11/XRandR bindings | `bd-a279ed` |
| `list` | Linux/X11 | `xprop` | Root window client list plus per-window metadata | **Bug** | Replace with native X11/EWMH property access | `bd-a279ed` |
| `list` | Linux/X11 | `xwininfo` | Window geometry/title fallback | **Bug** | Replace with native X11 geometry/title queries | `bd-a279ed` |
| `list` | Linux/Wayland | `hyprctl` | Hyprland monitor/client discovery | Documented backend prerequisite today | Prefer compositor-native bindings or another embedded backend if a stable option exists | `bd-e4edee` |
| `list` | Linux/Wayland | `swaymsg` | sway output/tree discovery | Documented backend prerequisite today | Prefer compositor-native bindings or another embedded backend if a stable option exists | `bd-e4edee` |
| `list` | Linux/Wayland | `wlr-randr` | wlroots output enumeration fallback | Documented backend prerequisite today | Evaluate portal/native alternatives for supported matrices | `bd-e4edee` |
| `list` | Windows 11 | `powershell` | Display and window discovery scripts | **Bug** | Replace with native Win32 bindings for monitor/window enumeration | `bd-a3357b` |
| `capture` | macOS | `screencapture` | Window/display PNG capture | Documented platform prerequisite | Replace with native ScreenCaptureKit/Quartz capture when ready for tighter packaging control | audited in `bd-d513d4` |
| `capture` | Linux/X11 | `import` | Window/root capture via ImageMagick | **Bug** | Replace with native X11/XComposite/XShm/XCB capture path or a vended helper strategy | `bd-a279ed` |
| `capture` | Linux/Wayland | `grim` | Compatibility fallback when portal screenshot capture is unavailable | Documented compatibility fallback | Keep the portal-backed path primary; retain `grim` only as a clearly diagnosed fallback for supported compositor families | `bd-e4edee` |
| `capture` | Windows 11 | `powershell` | Display/window capture scripts using Win32 APIs through PowerShell | **Bug** | Replace with native Win32/GDI/Windows Graphics Capture bindings | `bd-a3357b` |
| `run` | macOS | `osascript` | Focus transfer by PID/app name, text entry, key events, mouse clicks/drags via JXA/AppleScript | Documented platform prerequisite | Native accessibility/input bindings would still reduce subprocess overhead, but packaged usability no longer depends on the Swift toolchain | `bd-5c3937` |
| `run` | Linux/X11 | `xdotool` | Focus transfer, text entry, key input, mouse input | **Bug** | Replace with native X11/XTest input injection | `bd-a279ed` |
| `run` | Linux/Wayland | _none_ | Generic Wayland input is intentionally unsupported | Not applicable | Keep explicit unsupported-capability reporting until a compositor-specific backend exists | `bd-e4edee` |
| `run` | Windows 11 | `powershell` | Focus transfer, SendKeys text/key dispatch, mouse input | **Bug** | Replace with native Win32 input/focus APIs | `bd-a3357b` |

## Classification summary

### Acceptable documented prerequisites for v0.0.1

These still deserve future native work, but they do not currently require
operators to install extra package-manager tooling on a stock supported host:

- macOS `screencapture`
- macOS `osascript`
- Wayland compositor utilities (`hyprctl`, `swaymsg`, `wlr-randr`) when Tendril
  is explicitly operating against those compositor families
- Linux/Wayland `grim` only as an explicitly documented compatibility fallback
  after the preferred xdg-desktop-portal screenshot path has been tried

Why they are only *conditionally* acceptable:

- they are still subprocess boundaries rather than embedded implementations,
- they can still degrade startup latency and diagnostics, and
- they still need better operator-facing documentation so packaged-binary users
  know what environment Tendril expects.

### Self-containment/usability bugs

These dependencies materially undermine the “download one binary and run it”
goal:

- Linux/X11 `xrandr`, `xprop`, `xwininfo`, `import`, and `xdotool` because they
  are package-manager extras on many operator machines (`bd-a279ed`)
- Linux/Wayland capture backend availability remains a packaging/documentation
  concern because operators still need either a working xdg-desktop-portal
  screenshot backend or the documented `grim` fallback for some sessions
  (`bd-e4edee`)
- Windows `powershell` because the binary still outsources its core runtime
  surface to an external scripting host instead of embedding Win32 bindings
  (`bd-a3357b`)

## Prioritized native/embedded follow-up work

1. **Linux/X11 helper-chain removal next** — `bd-a279ed` covers the broadest set
   of third-party package prerequisites across discovery, capture, and input.
2. **Windows PowerShell removal** — `bd-a3357b` should move discovery, capture,
   and input to native Win32-backed Rust code for true binary self-containment.
3. **Wayland capture/backend hardening** — `bd-e4edee` now treats compositor-
   coupled discovery as a documented matrix, prefers xdg-desktop-portal for
   capture, and retains `grim` only as a compatibility fallback with clearer
   diagnostics for missing session tools.

## Operator-facing documentation gaps found by this audit

The current docs mention some platform expectations but do not yet provide one
source-backed page that answers all of these practical questions:

- which Tendril commands spawn host tools,
- which of those tools are expected on a stock OS versus needing extra install,
- which missing-tool failures are known product bugs rather than user mistake,
  and
- which follow-up beads are intended to eliminate the dependency.

The dedicated operator validation guides, especially
[`../linux-wayland-operator-validation.md`](../linux-wayland-operator-validation.md),
are the initial corrective coverage for that gap.
