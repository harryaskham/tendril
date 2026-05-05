# Platform support and rollout status

Tendril is designed around a platform adapter boundary so the CLI and MCP surface stay stable while platform-specific capture and input implementations evolve.

## Current shape in this repository

| Capability | CLI surface | Current status |
| --- | --- | --- |
| Target discovery | `list` | Implemented |
| Screenshot capture | `capture` | Implemented |
| Input execution | `run` | Implemented |
| Accessibility element discovery | `list-elements` | Implemented with macOS AX, Linux/X11 AT-SPI + surface fallback, Linux/Wayland AT-SPI, and Windows native Win32 control/window enumeration |
| MCP stdio | `mcp stdio` | Implemented for list/capture/run/list-elements |
| Audio capture | `listen` | WAV capture via `pw-record`/`parecord` (Linux) and `afrecord` (macOS); probe-only fallback elsewhere |
| Shell helper generation | `alias` | Implemented |
| WSL Windows-host tunnel | `--wsl-tunnel` | Implemented as a stateless proxy from WSL/Linux to a Windows-visible `tendril.exe`, including JSON/MCP stream preservation and clear setup errors |

## Notes

- Discovery currently focuses on windows and displays. On macOS there is no user-selected display socket analogous to X11 or Wayland; Tendril identifies the native WindowServer session as `mac_os_window_server` and discovers display/window connection details through Quartz/AppKit (`NSScreen` and `CGWindowListCopyWindowInfo`).
- `listen` writes WAV bytes to a temp file (or `--output <path>`) on Linux and macOS; the JSON envelope reports the on-disk path under `execution.artifact`. Windows and unrecognized backends still return `status = "probe_only"`.
- Linux/Wayland is a documented backend matrix rather than one generic path: Hyprland discovery uses `hyprctl`, sway uses `swaymsg`, wlroots display fallback uses `wlr-randr`, capture prefers xdg-desktop-portal with `grim` retained only as a compatibility fallback, and `list-elements` uses AT-SPI when applications publish accessibility metadata.
- Windows 11 support is native inside the Tendril binary for discovery, capture, input, and Win32 window/control element enumeration; audio capture remains probe-only pending a WASAPI recording backend.
- WSL tunnel mode (`--wsl-tunnel`) strips only the local tunnel flag and executes the same Tendril invocation through `tendril.exe` (or `TENDRIL_WSL_WINDOWS_BIN`) so WSL callers and remote Linux-to-WSL flows can target the Windows host while preserving standard JSON/MCP envelopes.
- The dedicated [Linux Wayland operator validation](../linux-wayland-operator-validation.md) guide covers those supported matrices explicitly.
- The docs site intentionally documents both fully implemented features and probe-first surfaces so the published contract matches the repository state.
- For a source-backed inventory of runtime subprocess/tool dependencies and their current self-containment classification, see [Runtime dependency audit](runtime-dependencies.md).
