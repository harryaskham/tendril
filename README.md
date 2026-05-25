# Tendril

Tendril is a stateless Rust CLI for agent-driven desktop inspection and control.
It can:

- list window and display targets,
- capture screenshots from a chosen window or display,
- run text or input-sequence actions against a target,
- expose the same `list`, `capture`, and `run` surface over MCP stdio, and
- probe audio-capture capability with `listen`.

There is no Tendril daemon and no hidden runtime store. Each command is
self-contained.

## Primary references

- [Approved spec](SPEC.md)
- [Project health / handoff summary](PROJECT_HEALTH.md)
- [Published docs source](docs/src/index.md)
- [Changelog](CHANGELOG.md)

## Enter the development environment

This repo is set up for Nix + direnv:

```bash
direnv allow
nix develop
```

`.envrc` uses `use flake`, so entering the repo loads the dev shell from
`flake.nix`. If your shell does not auto-load direnv yet, run:

```bash
eval "$(direnv export bash)"
```

The dev shell includes Rust, cargo, clippy, rustfmt, rust-analyzer, direnv, and
Nix formatting tools.

Useful repo-local commands:

```bash
./scripts/bootstrap.sh                 # initialize submodules, install hooks, allow/load direnv
nix build .#tendril .#mcp-cli          # build workspace packages
./scripts/pre-merge.sh                 # fmt + clippy + tests + flake checks
nix develop --command ./scripts/build-docs.sh
```

Android device smoke examples:

```bash
tendril --android auto list --json
tendril --android sgu24:5555 list-elements --json
tendril --android sgu24:5555 capture -o android.png
tendril --android sgu24:5555 run 'press("launch:com.example.app"),wait(1s),click(231,1905),Back'
```

See [the Android backend reference](docs/src/reference/android.md) for supported ADB/UIAutomator capabilities and safety limits.

## Workspace layout

- `crates/tendril`: the Tendril CLI
- `crates/mcp-cli`: reusable JSON-envelope and MCP stdio support, pinned from `https://github.com/harryaskham/mcp-cli`; the workspace package and Tendril dependency stay on the same upstream revision so the shared `updatable-cli` MCP extension can reuse the same `mcp-cli` types
- `docs/`: mdBook-based documentation site source
- `flake.nix`: dev shell, packages, checks, and reproducible build outputs
- `.cacophony/config.yaml`: project bootstrap plus managed build/test defaults
- `.cacophony/project.yaml`: managed bootstrap/build/test/lint/pre-merge actions
- `scripts/`: bootstrap, validation, docs, and release helpers
- `PROJECT_HEALTH.md`: operator-facing repo-health audit summary and follow-ups

## Stateless runtime and local config

Tendril does not keep session state between commands.

The only machine-local runtime state is the config file:

- `$TENDRIL_CONFIG_DIR/config.yaml`, if `TENDRIL_CONFIG_DIR` is set
- otherwise `$XDG_CONFIG_HOME/tendril/config.yaml`
- otherwise `~/.config/tendril/config.yaml`

Current config fields are capture, logging, and execution-lock defaults:

```yaml
capture:
  format: png        # png or jpeg
  compression: 85    # 0-100
  max_width: 1440    # optional
  max_height: 900    # optional
  timeout_ms: null   # optional backend deadline
logging:
  level: info        # error, warn, info, debug, trace
execution_lock:
  enabled: true      # serialize tendril run by default
  timeout_ms: 60000  # queue wait timeout
  stale_ms: 30000    # stale heartbeat threshold
  path: null         # optional lock root override
```

If the file is missing, Tendril uses built-in defaults. Alias helpers are also
stateless: `tendril alias` prints shell code, but Tendril does not store alias
state itself.

## Agent workflow: list -> capture -> run

The intended workflow is:

1. discover targets with `list`
2. choose a window or display id
3. capture current state with `capture`
4. execute text or an input DSL sequence with `run`

For browser/OS automation that must not touch the operator's real desktop, use
the isolated 1920x1080 Xvfb micro-environment documented in
[docs/src/headless-micro-environment.md](docs/src/headless-micro-environment.md):

```bash
cargo build -p tendril
scripts/tendril-headless.sh --name smoke --tendril-bin ./target/debug/tendril smoke
```

Smoke captures are written under `summaries/$CACOPHONY_AGENT/` by default so
Cacophony summaries and `/tmp/watch-captures.sh` can surface them. The smoke
path requires a discovered browser window and writes a browser-after capture
showing Tendril-controlled input; XTerm or window-manager helper windows are not
accepted as browser-control proof. Firefox file uploads in this headless desktop
use the explicit `firefox-upload`/`file-upload-smoke` helper because the native
Firefox chooser is not exposed as a separate Tendril target after clicking a file
input. The Nix package also exposes the helper; run `nix run .#tendril-headless -- smoke`.

Examples:

```bash
# List windows and displays

tendril --json list

# List windows and displays on a remote desktop over SSH.
# Linux remotes auto-discover X11/Wayland session variables when SSH did not inherit them.

tendril --remote me@box --json list

# Capture a window using config defaults

tendril --json --window <window-id> capture

# Capture a display with explicit resize + jpeg settings

tendril --json --display <display-id> capture \
  --max-width 1440 \
  --format jpeg \
  --compression 80

# Save a capture directly to a file with -o/--output
# (works with or without --json; --json still prints the envelope to stdout)

tendril --window <window-id> capture -o /tmp/screen.png
tendril --json --window <window-id> capture -o /tmp/screen.png

# Type text or run the input DSL against a target.
# `run` waits on the host-local execution lock/queue by default.

tendril --json --window <window-id> run 'send("hello")'
tendril --json --window <window-id> run 'send("hello"),Return'
tendril --json --window <window-id> run 'hold(ctrl),c,release(ctrl),wait(1s),send("done")'
tendril --json --window <window-id> run 'dblclick(320,240),wait(250ms)'
tendril --json --window <window-id> run 'scroll(220,420,8),wait(250ms)'
tendril --json --window <window-id> run --lock-timeout-ms 5000 'send("bounded wait")'
tendril --json --window <window-id> run --no-lock 'send("advanced opt-out")'

# Ambiguous command-looking single segments are rejected instead of typed literally.
# Use send("...") for text and a comma-separated DSL sequence for key taps.

tendril --json --window <window-id> run 'Return'      # invalid_run_input with hint
tendril --json --window <window-id> run 'type "hi"'  # invalid_run_input with hint

# On Linux/X11 browser windows, avoid Ctrl+L URL navigation: Firefox can keep
# focus inside a page input. Capture, click the visible address bar, type, then
# recapture/verify instead.

tendril --json --window <browser-id> run \
  'lclick(<address_bar_x>,<address_bar_y>),hold(ctrl),a,release(ctrl),send("https://example.com"),Return,wait(1000ms)'

# Emit a reusable shell wrapper for a target (shell state, not Tendril state)

eval "$(tendril --window <window-id> alias --name desk)"
```

Target-scoped commands use the global `--window <id>` or `--display <id>`
flags. `capture`, `run`, and `alias` require exactly one target selector.

`--remote user@host` proxies the same invocation over `ssh`, strips only the
local `--remote` flag, bootstraps common non-login-shell `PATH` entries, and
then execs `tendril` on the remote host. `--wsl-tunnel` proxies the invocation
from WSL/Linux to a Windows-host `tendril.exe`; it also composes with `--remote`
because the flag is forwarded to the remote Tendril process. If no Windows
binary is visible, the WSL tunnel downloads the latest Windows release asset,
verifies its checksum, and installs `tendril.exe` under
`%LOCALAPPDATA%\\Tendril\\bin` for reuse. Set `TENDRIL_WSL_WINDOWS_BIN` when the
Windows executable lives somewhere else, or `TENDRIL_WSL_INSTALL_DIR` to choose
the auto-install directory. Set `TENDRIL_REMOTE_BIN` remotely when
the binary is not named `tendril` or is outside `PATH`. On Linux remotes, the
bootstrap prefers existing graphical environment variables; if SSH did not
provide them, it discovers `/run/user/<uid>/wayland-*`, `/tmp/.X11-unix/X*`,
`XDG_RUNTIME_DIR`, and the session bus so macOS, X11, and Wayland desktops can
be listed/captured/run transparently. When a Linux display socket is inferred,
the remote process also receives `TENDRIL_DISCOVERED_X11_SOCKET` or
`TENDRIL_DISCOVERED_WAYLAND_SOCKET` for diagnostics while continuing to use the
standard display variables.

## JSON mode

`--json` is a global flag on the CLI. In JSON mode, Tendril emits structured
success and error envelopes so agents do not need to scrape human text.

Typical pattern:

```bash
tendril --json list
tendril --json --window <window-id> capture
tendril --json --window <window-id> run 'send("hello")'
```

Successful responses include a command name in `meta.command` and command data
under `data`. Errors are also structured and categorized instead of being
opaque plain-text failures.

## MCP stdio

Run Tendril as an MCP server with:

```bash
tendril mcp stdio
```

The initial MCP tool set is:

- `list`
- `capture`
- `run`

The MCP arguments match the CLI command model:

- `list`: `{}`
- `capture`: `{ "window": "..." }` or `{ "display": "..." }` plus optional
  `max_width`, `max_height`, `format`, and `compression`
- `run`: `{ "window": "...", "input_definition": "send(\"hello\")" }`
  or the display-scoped equivalent

Minimal stdio flow:

1. start `tendril mcp stdio`
2. send `initialize`
3. send `tools/list`
4. call `tools/call` for `list`, `capture`, or `run`

Example tool calls:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capture","arguments":{"window":"window-1"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run","arguments":{"window":"window-1","input_definition":"send(\"hello\")"}}}
```

CLI JSON mode and MCP tool responses share the same structured payload shape.

For Pi and Cacophony consumers, the Tendril-side launch/session/tool contract is documented in [docs/src/reference/pi-cacophony-mcp-contract.md](docs/src/reference/pi-cacophony-mcp-contract.md). A raw external-client smoke probe is available via:

```bash
./scripts/mcp-stdio-smoke.sh -- nix run .#tendril -- mcp stdio
```

## macOS operator validation

If you want to validate Tendril on macOS without reading the codebase, run these examples from the repository root:

```bash
# 1) List targets
nix run .#tendril -- list
nix run .#tendril -- list --json

# 2) Capture after copying a display or window id from list output
nix run .#tendril -- --display <display-id> capture --json --max-width 1440 > /tmp/tendril-capture.json
nix run .#tendril -- --window <window-id> capture --json > /tmp/tendril-window-capture.json

# 3) Run input against a harmless target such as TextEdit
nix run .#tendril -- --window <window-id> run --json 'send("hello from Tendril on macOS")'

# 4) Launch MCP stdio
nix run .#tendril -- mcp stdio
```

What to expect on macOS:

- `list` and `capture` should succeed once **Screen Recording** is granted.
- `run` should succeed once **Accessibility** is granted.
- The permission entries usually appear under **System Settings > Privacy & Security** for the invoking terminal app or Tendril binary.
- If you see `missing_permission` in JSON mode, the error should tell you which privacy setting to enable.

Representative Screen Recording failure for `list`:

```json
{
  "status": "error",
  "error": {
    "category": "missing_permission",
    "code": "missing_permission",
    "message": "macOS target discovery needs Screen Recording consent to enumerate visible windows."
  }
}
```

If you instead see another macOS runtime-tool failure, the command should surface a structured Tendril error rather than requiring a separate developer toolchain. Use the dedicated macOS operator validation guide below for current troubleshooting steps.

For the full operator-facing guides, including packaged-binary smoke checks and troubleshooting detail, see:

- [docs/src/macos-operator-validation.md](docs/src/macos-operator-validation.md)
- [docs/src/linux-x11-operator-validation.md](docs/src/linux-x11-operator-validation.md)
- [docs/src/headless-micro-environment.md](docs/src/headless-micro-environment.md)

## Platform and permission expectations

Tendril expects to run inside an active local desktop session. It does not
start a helper daemon to bypass platform rules.

### macOS

- `list`/`capture`/`run` are intended for a local GUI session.
- Target discovery and input dispatch use built-in macOS Quartz/AppKit paths via
  `osascript`/JXA rather than a runtime Swift toolchain, and can coexist with
  yabai/skhd-managed desktops while keeping Quartz discovery authoritative.
- Screen capture requires **Screen Recording** consent for the invoking terminal
  or Tendril binary.
- Input control requires **Accessibility** consent.
- Microphone probing/capture paths require **Microphone** consent.
- System loopback audio is not exposed by the current macOS adapter.

### Linux

- Tendril expects an active graphical session.
- The generic adapter supports discovery/capture/input on **X11** via an embedded X11/XRandR/XTest backend rather than external helper tools.
- On **Wayland**, Tendril supports a documented backend matrix: target
  discovery uses Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output
  enumeration (`wlr-randr`) depending on the active compositor family.
- Wayland capture prefers `xdg-desktop-portal` screenshot backends and only
  uses `grim` as a compatibility fallback when the portal path is unavailable.
- Wayland input injection remains compositor-specific and is not exposed by the
  generic adapter surface.
- For explicit operator validation steps and expected backend failures, see
  [docs/src/linux-x11-operator-validation.md](docs/src/linux-x11-operator-validation.md)
  and [docs/src/linux-wayland-operator-validation.md](docs/src/linux-wayland-operator-validation.md).
- Audio probing expects a supported user-session backend such as PipeWire or
  PulseAudio.
- Linux permissions are usually session/backend constraints rather than central
  OS privacy prompts.

### Windows 11

- Tendril expects a normal desktop user session.
- Discovery, capture, input, and `list-elements` use native Win32 APIs inside
  the Tendril binary and do not require a Tendril-managed background service.
- `list-elements` exposes a pragmatic Win32 window/control tree with stable
  snapshot-local IDs that can be passed to `run 'click(<id>)'`.
- Microphone paths may depend on **Settings > Privacy & security > Microphone**
  for desktop apps.

For a source-backed inventory of the current runtime subprocess/tool surface and
its self-containment classification, see
[`docs/src/reference/runtime-dependencies.md`](docs/src/reference/runtime-dependencies.md).

## Audio capture status

`tendril listen` now performs a real WAV recording on supported backends and
falls back to probe-only diagnostics elsewhere:

- **Linux + PipeWire** uses `pw-record` (with `parecord` as a fallback).
- **Linux + PulseAudio** uses `parecord` against `@DEFAULT_MONITOR@` /
  `@DEFAULT_SOURCE@`.
- **macOS** uses `afrecord` (Apple's CoreAudio-backed recorder shipped with
  the OS).
- **Windows / unknown backends** continue to return a structured
  `status = "probe_only"` envelope with a note explaining the gap.

The captured WAV is written either to an explicit `--output <path>` (mirrors
`capture -o`) or to a temp file allocated by `listen`; the path is included
in the JSON envelope under `execution.artifact`. The probe-first capability
and permission diagnostics from the previous slice are still emitted alongside
the artifact so callers retain backend, channel, and consent metadata.

Documented gaps:

- `--format flac` and `--format opus` are accepted by the surface but
  currently degrade to probe-only; only WAV is emitted today.
- Explicit `device:<id>` binding is accepted by the command surface but
  returns a structured unsupported-capability result until adapter-specific
  device enumeration/binding lands.

## Release automation

Local pre-merge validation remains the primary fast-feedback gate:

```bash
./scripts/pre-merge.sh
```

Tendril uses SemVer. The release version comes from `[workspace.package].version`
in `Cargo.toml`, and release tags use the `v<semver>` form, for example
`v0.0.1`. Use the built-in bump helper to update all versioned manifests and
create the release commit:

```bash
tendril version bump patch   # or: minor, major
```

Users can install or update from GitHub release assets with:

```bash
tendril update                         # installs latest matching platform binary to ~/.local/bin
tendril update --dry-run               # shows the planned latest-release query and install path
tendril update --release-version 0.0.1 # installs a specific release version
```

The MCP stdio server also registers the shared `updatable-cli` tools
`self_update_status`, `self_update_check`, and `self_update_run`, following the
same reference pattern used by ring-mods. MCP clients can therefore inspect and
apply Tendril binary updates without bespoke Tendril update wiring.

Pushing a `v*` tag or landing a commit that changes the workspace version on
`main` starts the GitHub Actions release workflow. The workflow reruns pre-merge
validation, builds Linux artifacts on `[self-hosted, linux]`, builds macOS
artifacts on `[self-hosted, macos]`, stages the combined asset set, and publishes
a GitHub release for the matching `v<semver>` tag.

The release asset set is intended to include at least:

- `tendril-<semver>-x86_64-linux.tar.gz`
- `tendril-<semver>-x86_64-linux.sha256`
- `tendril-<semver>-aarch64-darwin.tar.gz`
- `tendril-<semver>-aarch64-darwin.sha256`
- `tendril-<semver>-source.tar.gz`
- `tendril-<semver>-source.sha256`
- `release-manifest.json`

Useful local release helpers:

```bash
./scripts/stage-release-artifacts.sh v0.0.1
./scripts/release-artifacts.sh v0.0.1
./scripts/release-notes.sh v0.0.1
```

## Documentation site

The repository publishes a static docs site built from `docs/`.

- local build: `nix develop --command ./scripts/build-docs.sh`
- mdBook source: `docs/src/`
- published Pages artifact: `target/book/`
- generated Rust API docs: `target/book/api/`
- deployment workflows: `.github/workflows/pages.yaml` and `.github/workflows/tag-release.yml`

## Notes for developers and agents

- Running `tendril` with no arguments prints agent-oriented help.
- The workspace version target is `0.0.1`.
- Remaining handoff follow-ups are captured explicitly in [PROJECT_HEALTH.md](PROJECT_HEALTH.md).
