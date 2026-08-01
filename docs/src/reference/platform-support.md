# Platform support and rollout status

Tendril is designed around a platform adapter boundary so the CLI and MCP surface stay stable while platform-specific capture and input implementations evolve.

## Current shape in this repository

| Capability | CLI surface | Current status |
| --- | --- | --- |
| Target discovery | `list` | Implemented |
| Screenshot and camera-frame capture | `capture` | Implemented; cameras use AVFoundation (macOS), V4L2 (Linux), or DirectShow (Windows) through ffmpeg |
| Input execution | `run` | Implemented |
| Accessibility element discovery | `list-elements` | Implemented with macOS AX, Linux/X11 AT-SPI + surface fallback, Linux/Wayland AT-SPI, and Windows native Win32 control/window enumeration |
| MCP stdio | `mcp stdio` | Implemented for list/capture/run/list-elements |
| Audio capture | `listen` | WAV capture via `pw-record`/`parecord` (Linux) and `afrecord` (macOS); probe-only fallback elsewhere |
| Shell helper generation | `alias` | Implemented |
| WSL Windows-host tunnel | `--wsl-tunnel` | Implemented as a stateless proxy from WSL/Linux to a Windows-visible `tendril.exe`, including JSON/MCP stream preservation and clear setup errors |

## Notes

- Discovery includes windows, displays, and cameras. macOS camera metadata comes from `system_profiler`, Linux camera nodes from V4L2 sysfs, and Windows camera names/ids from ffmpeg's DirectShow inventory. On macOS there is no user-selected display socket analogous to X11 or Wayland; Tendril identifies the native WindowServer session as `mac_os_window_server` and discovers display/window connection details through Quartz/AppKit (`NSScreen` and `CGWindowListCopyWindowInfo`).
- `listen` writes WAV bytes to a temp file (or `--output <path>`) on Linux and macOS; the JSON envelope reports the on-disk path under `execution.artifact`. Windows and unrecognized backends still return `status = "probe_only"`.
- Linux/Wayland is a documented backend matrix rather than one generic path: Hyprland discovery uses `hyprctl`, sway uses `swaymsg`, wlroots display fallback uses `wlr-randr`, capture prefers xdg-desktop-portal with `grim` retained only as a compatibility fallback, and `list-elements` uses AT-SPI when applications publish accessibility metadata.
- Windows 11 support is native inside the Tendril binary for discovery, capture, input, and Win32 window/control element enumeration; audio capture remains probe-only pending a WASAPI recording backend.
- WSL tunnel mode (`--wsl-tunnel`) strips only the local tunnel flag and executes the same Tendril invocation through `tendril.exe` (or `TENDRIL_WSL_WINDOWS_BIN`) so WSL callers and remote Linux-to-WSL flows can target the Windows host while preserving standard JSON/MCP envelopes.
- The dedicated [Linux Wayland operator validation](../linux-wayland-operator-validation.md) guide covers those supported matrices explicitly.
- The docs site intentionally documents both fully implemented features and probe-first surfaces so the published contract matches the repository state.
- For a source-backed inventory of runtime subprocess/tool dependencies and their current self-containment classification, see [Runtime dependency audit](runtime-dependencies.md).

## Native Windows validation

Windows 11 is a first-class supported platform, but the primary CI workflow
(`.github/workflows/ci.yml`) only runs `nix flake check` on the self-hosted
NixOS Linux runners, so it never compiles the Windows-only code
(`crates/tendril-win32` and the `#[cfg(windows)]` /
`#[cfg(target_os = "windows")]` blocks in `crates/tendril`). To keep those code
paths from rotting between releases, a dedicated `windows` workflow
(`.github/workflows/windows.yml`) builds the whole workspace on a
GitHub-hosted `windows-latest` runner (default `x86_64-pc-windows-msvc`
toolchain) on every pull request and `main` push, plus a `--version` smoke of
the packaged binary.

- The `windows` workflow is intentionally **separate from `ci.yml` and is not a
  required PR-merge status check**, so a slow or flaky GitHub-hosted Windows
  runner can never block the tendril merge path. Promote it to a required check
  once it has a stable green history.
- The `windows-latest` runner is **headless** (no interactive desktop), so the
  smoke is limited to `--version`; a real desktop smoke of
  `list`/`capture`/`run`/`list-elements` remains a follow-up that needs a
  self-hosted Windows desktop runner.
- To reproduce the compile check locally from a non-Windows checkout, install
  the cross target and run a check, for example
  `rustup target add x86_64-pc-windows-gnu` followed by
  `cargo check --workspace --target x86_64-pc-windows-gnu` (the GNU target needs
  a MinGW-w64 toolchain; the CI job uses the native MSVC target instead).

## Native macOS validation

macOS is also a first-class supported platform (AppKit/Quartz discovery, the
JXA accessibility path, `afrecord` audio capture, macOS release artifacts), but
like Windows it had no per-change CI — `ci.yml` builds only on x86_64-linux and
the only macOS build was `tag-release.yml`'s `build-macos`, which runs solely on
release tags. A dedicated `macos` workflow (`.github/workflows/macos.yml`) now
builds tendril on the self-hosted `aarch64-darwin` runner (`tendril-ms-mac`, via
`nix build .#tendril`) on every pull request and `main` push, plus a
`--version` smoke of the built binary.

- Like the `windows` workflow, `macos` is **separate from `ci.yml` and is not a
  required PR-merge status check**, so a busy or offline self-hosted macOS
  runner can never block the tendril merge path. Promote it to a required check
  once it has a stable green history.
- It validates that tendril's unix dependencies (`zbus`/`ashpd`/`x11rb`) and the
  macOS code paths compile on darwin; the `--version` smoke needs no desktop
  session.
- Together, `ci.yml` (Linux), `windows.yml`, and `macos.yml` give per-change
  build coverage across all three supported platforms.
