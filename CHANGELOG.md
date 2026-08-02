# Changelog

All notable changes to Tendril are documented in this file.
Format based on [Keep a Changelog](https://keepachangelog.com/).

Tendril follows [Semantic Versioning](https://semver.org/). Release notes are cut from SemVer tags in the form `vX.Y.Z`, and the `Unreleased` section tracks changes on `main` until the next tag is created.

## [Unreleased]

### Fixed
- Release checksum staging no longer assumes `awk` is available in the minimal self-hosted runner PATH; shell-only digest extraction keeps tag repair dispatches portable across the release fleet (bd-598fb2).
- The release matrix now uses the repository's real `aarch64-darwin` runner label and cross-compiles a static `aarch64-linux` musl binary inside the tag's dev shell on a self-hosted x86_64 Nix runner, because no native aarch64 Linux Actions runner is registered (bd-11731d).

## [0.0.4] - 2026-08-01

### Added
- Cross-platform camera discovery and still-frame capture through AVFoundation on macOS, V4L2 on Linux, and DirectShow on Windows. macOS explicitly negotiates 30 fps (with an advertised-rate retry), fixing Logitech C930e capture failures caused by ffmpeg's 29.97 fps default (bd-ad42e8).
- Repository-root `cargo install --path .` support; the root manifest is now the Tendril package as well as the workspace manifest (bd-8b07a5).

### Fixed
- `tendril version` now prints the running binary version and no longer exposes the repository-mutating `version bump` development tool (bd-8b07a5).
- `tendril update` now delegates status, release checks, checksum verification, staging, and promotion directly to `updatable-cli` instead of maintaining a second installer implementation (bd-8b07a5).
- Tag releases now package raw Cargo binaries from the Nix dev shell on self-hosted x86_64-linux, aarch64-linux, and aarch64-darwin runners. The release path refuses Darwin binaries with Nix-store linkage and no longer ships the non-portable `.tendril-wrapped` Nix wrapper (bd-8b07a5).

## [0.0.3] - 2026-05-17

### Fixed
- macOS release archives now match the Tendril/updatable-cli self-update contract by packaging binaries under `tendril-<version>-<target>/` instead of at the tarball root. This lets `tendril update` and the MCP `self_update_run` tool find and verify the downloaded binary on macOS (bd-3d3f4b).

## [0.0.2] - 2026-05-17

### Added
- Tendril MCP stdio now exposes the shared `updatable-cli` self-update tools: `self_update_status`, `self_update_check`, and `self_update_run`. This matches the ring-mods reference pattern so long-running MCP clients can inspect and apply Tendril binary updates dynamically without bespoke Tendril-specific update wiring (bd-91b7f5).

### Fixed
- Wayland capture now reserves part of the command deadline for the `grim` fallback when that backend is available. If `xdg-desktop-portal` advertises a screenshot path but hangs waiting on an invisible compositor/permission dialog, Tendril abandons the portal-first attempt promptly and captures through `grim` instead of spending the whole default timeout before the fallback can run (bd-d522bf).
- macOS `tendril list` again enumerates open windows alongside displays. The JXA discovery script's `CGWindowListCopyWindowInfo` call returns a `CFArrayRef`; `ObjC.deepUnwrap` already wraps it via `ObjC.castRefToObject`, but there was no regression test pinning the cast in place. A previous build deployed to operator macOS hosts shipped an older script that fed the raw `CFArrayRef` into `deepUnwrap` and silently produced an empty array, so `tendril list --json` returned only `kind=display` entries and `tendril --window <id>` was unreachable from remote agents. Added a regression test that asserts the discovery script wraps the CFArrayRef with `ObjC.castRefToObject` before `ObjC.deepUnwrap` and still emits `kind: 'window'` targets, so future edits cannot silently drop window enumeration on macOS again (bd-845b47).

### Changed
- macOS Screen Recording remediation guidance is now actionable end-to-end. The `missing_permission: screen_capture` diagnostic surfaced by `tendril capture` and `tendril list` (target discovery) now embeds the absolute path of the running tendril binary, the parent process name+pid (so operators invoking via SSH or `caco @ms-mac exec` know which launcher also needs TCC consent), the deep-link `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture` URL for the exact System Settings pane, and the numbered grant/relaunch/`tccutil reset ScreenCapture` recovery steps. `tendril list --json` additionally probes Screen Recording consent proactively (1×1 `screencapture -x -R 0,0,1,1` round-trip) and reports `state = granted | denied | unknown` on the `screen_capture` permission row instead of always returning `unknown`, so agents can detect a missing grant without first attempting a full capture (bd-a24d8d).
- `tendril run` on macOS no longer fails with a misleading permission error when System Events times out: the text-dispatch step now wraps the `keystroke` call in `with timeout of 300 seconds` (long enough to absorb a pending TCC consent prompt without hanging unbounded), and a distinct `input_command_timeout` error code is surfaced for `errAETimeout` (-1712) instead of misclassifying the failure as an Accessibility denial. The new code carries a `hint` detail pointing operators at confirming the consent prompt, restarting System Events, or retrying with `--window` (which uses CoreGraphics-only modifier/mouse paths) (bd-634cc7).
- `tendril run` DSL parsing now detects a top-level `;` (outside parentheses and string literals) and surfaces a structured `invalid_run_input` diagnostic with `stage = parse` and the message `unexpected `;`; the DSL separator is `,`` (plus the byte `offset` of the offending separator) instead of routing the input through downstream parsers that produced misleading errors like `y must be an integer in ...`. `;` inside `send("...")` strings or balanced parens is unaffected (bd-aa54ef).

### Added
- `tendril run` now uses a default host-local execution lock/queue so concurrent CLI or MCP input runs on the same host/session wait instead of interleaving. The run result reports `data.execution_lock` JSON metadata (wait time, queue depth, stale reaping, lock path, token), lock timeouts return structured `timeout/execution_lock_timeout` details, stale owner/ticket heartbeats are reaped, and advanced users can tune or opt out with `--no-lock`, `--lock-timeout-ms`, `--lock-stale-ms`, `--lock-path`, environment variables, or `execution_lock` config (bd-73e61e).
- `tendril listen` now performs a real WAV recording on supported backends instead of always returning `status = "probe_only"`. On Linux it shells out to `pw-record` (preferred when PipeWire is detected, with `parecord` as a fallback) or `parecord` (PulseAudio); on macOS it uses Apple's bundled `afrecord` (CoreAudio). Captured audio is written to `--output <path>` (mirrors `capture -o`) or to a temp file under `$TMPDIR`, and the JSON envelope reports the artifact path, byte size, sample rate, channels, and recorder under `execution.artifact`. Header-only outputs (silent sources) are flagged in `notes` so callers can detect mute/suspended sessions. Windows and unrecognized backends continue to return the structured probe-only envelope (bd-d7c2f0).
- Configurable per-call deadline for Wayland capture: `tendril capture --timeout-ms <ms>` (and the equivalent `capture.timeout_ms` config / MCP `timeout_ms` argument) bound the xdg-desktop-portal screenshot D-Bus call and the `grim` fallback subprocess. The portal request runs on a worker thread and is abandoned on deadline; the grim child is killed and reaped. A new `Timeout` error category surfaces a structured `platform_adapter_timeout` failure (with `operation`, `platform`, and `timeout_ms` details) so agents can recover instead of hanging forever. Default deadline is 10 000 ms (bd-aefc14).
- Wayland input injection support for Hyprland and other wlroots compositors via runtime detection of `ydotool` (preferred, full keyboard + pointer) and `wtype` (keyboard-only fallback). Wayland targets now report `input_supported = true` when at least one helper tool is on PATH, and `tendril run` dispatches the existing DSL (send/lclick/rclick/mclick/hold/release/wait/drag) through the detected backend with target-relative→absolute coordinate translation. Missing-helper paths surface a structured `unsupported_capability` diagnostic that names both tools and the `ydotoold` daemon (bd-408572).
- `ScaleFactor::new` constructor that reduces fractions to lowest terms (and clamps zero components) so display targets and window targets share the same canonical representation in `tendril list --json` (bd-e123b8).
- `PROJECT_HEALTH.md` handoff summary that links the spec, docs, validation, and release surfaces and captures explicit follow-ups.
- MIT `LICENSE` file and release-artifact packaging that now ships the license and project health summary alongside the changelog and README.
- A dedicated macOS operator-validation guide with copy-pasteable `nix run` examples for `list`, `capture`, `run`, and MCP stdio, plus permission-prompt expectations and self-containment troubleshooting.
- A published Pi/Cacophony-facing MCP integration contract that documents the `tendril mcp stdio` launch expectations, desktop-session and permission assumptions, stable tool names/arguments, and semver alignment with Tendril's MCP schemas.
- An external-client MCP smoke script and integration test that initialize Tendril over stdio, verify `tools/list` schema metadata, and call the `list` tool against the built binary contract.
- A Linux/X11 packaged-smoke script and operator guide for validating packaged `list`/`capture` flows, with optional real-input smoke coverage for `run`.

### Changed
- macOS `tendril run --window <id>` now raises the requested window before activating its application. Previously it called `NSRunningApplication.activateWithOptions` only, which brought the app forward but left whichever window was frontmost as the input target — sending input into the wrong window when an app had multiple windows open. The focus path now uses the Accessibility API (`AXUIElementCreateApplication` + `AXWindows` + `AXRaise`), matching by CGWindowID via the private `_AXUIElementGetWindow` helper when available and falling back to position+size matching against the discovery bounds. Accessibility lookup failures are non-fatal and the existing app-level activation still runs (bd-fc4aff).
- `tendril run` on Wayland now probes adapter-level input support before the per-target capability check so sessions missing both `ydotool` and `wtype` surface the actionable missing-backend diagnostic (which names both helpers and points at an install path) instead of the generic `input_not_supported_for_target` error (bd-da01d3).
- Wayland (Hyprland), sway, and X11 target discovery now filter out `xdg-desktop-portal-*` dialog windows so failed portal capture attempts cannot pollute `tendril list` output with stale authorization dialogs (bd-b6adf6).
- Windows 11 discovery, capture, and input no longer depend on spawning `powershell`; Tendril now uses embedded Win32 bindings for packaged-binary self-containment and covers the native flow with Windows-focused unit tests.
- README now links the approved spec, managed validation commands, runtime config location, docs publication surface, handoff health summary, and packaged macOS/Linux smoke-validation examples.
- Linux/X11 discovery, capture, and input now use an embedded X11/XRandR/XTest backend instead of `xrandr`, `xprop`, `xwininfo`, `import`, or `xdotool` helper tools.
- The runtime dependency audit now reflects the self-contained Linux/X11 path, the self-contained Windows path, and the remaining packaged-runtime follow-ups.
- Cargo package metadata now carries shared repository and homepage information for the workspace crates.
- Tag-triggered GitHub Actions release automation remains backed by the Nix flake and local pre-merge checks.
- Seeded the changelog and release-note flow so future releases can prepend human-readable summaries when a new `vX.Y.Z` tag is pushed.

## [0.0.1] - 2026-04-09

### Added
- Bootstrapped the Tendril Rust workspace at version `0.0.1`, including the `tendril` CLI crate and the in-repo reusable `mcp-cli` support crate.
- Added the initial agent-facing command surface: `tendril list`, `tendril capture`, `tendril run`, `tendril alias`, `tendril listen`, and `tendril mcp stdio`.
- Added structured JSON and MCP envelopes, typed command models, config loading from `~/.config/tendril/config.yaml`, and agent-oriented help for the list → capture → run workflow.
- Added target discovery, screenshot capture with resize/remapping metadata, target-scoped input execution with DSL support, and probe-first audio capability diagnostics.
- Added cross-platform adapter scaffolding for macOS, Linux, and Windows 11 with explicit capability, permission, and structured error reporting.
- Added Nix flake packaging, reproducible checks, Cacophony project bootstrap, git hooks, and the local `scripts/pre-merge.sh` validation gate.
- Added integration, CLI/MCP parity, and platform contract test coverage for the initial stateless desktop automation workflow.
- Added explicit SemVer and repository metadata wiring in Cargo manifests and release documentation.
- Added reproducible `.#releaseArtifact` packaging plus local release helper scripts for canonical binary archives, checksums, and manifest metadata.
