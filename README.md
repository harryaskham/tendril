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
./scripts/bootstrap.sh                 # install hooks + allow/load direnv
nix build .#tendril .#mcp-cli          # build workspace packages
./scripts/pre-merge.sh                 # fmt + clippy + tests + flake checks
nix develop --command ./scripts/build-docs.sh
```

## Workspace layout

- `crates/tendril`: the Tendril CLI
- `crates/mcp-cli`: reusable JSON-envelope and MCP stdio support
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

Current config fields are capture and logging defaults:

```yaml
capture:
  format: png        # png or jpeg
  compression: 85    # 0-100
  max_width: 1440    # optional
  max_height: 900    # optional
logging:
  level: info        # error, warn, info, debug, trace
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

Examples:

```bash
# List windows and displays

tendril --json list

# Capture a window using config defaults

tendril --json --window <window-id> capture

# Capture a display with explicit resize + jpeg settings

tendril --json --display <display-id> capture \
  --max-width 1440 \
  --format jpeg \
  --compression 80

# Type text or run the input DSL against a target

tendril --json --window <window-id> run 'send("hello")'
tendril --json --window <window-id> run 'hold(ctrl),c,release(ctrl),wait(1s),send("done")'

# Emit a reusable shell wrapper for a target (shell state, not Tendril state)

eval "$(tendril --window <window-id> alias --name desk)"
```

Target-scoped commands use the global `--window <id>` or `--display <id>`
flags. `capture`, `run`, and `alias` require exactly one target selector.

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

For the full operator-facing guide, including a copy-pasteable raw MCP `tools/list` probe and more troubleshooting detail, see [docs/src/macos-operator-validation.md](docs/src/macos-operator-validation.md).

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
- The generic adapter supports discovery/capture/input on **X11**.
- On **Wayland**, Tendril supports a documented backend matrix: target
  discovery uses Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output
  enumeration (`wlr-randr`) depending on the active compositor family.
- Wayland capture now prefers `xdg-desktop-portal` screenshot backends and only
  uses `grim` as a compatibility fallback when the portal path is unavailable.
- Wayland input injection remains compositor-specific and is not exposed by the
  generic adapter surface.
- For explicit operator validation steps and expected backend failures, see
  [docs/src/linux-wayland-operator-validation.md](docs/src/linux-wayland-operator-validation.md).
- Audio probing expects a supported user-session backend such as PipeWire or
  PulseAudio.
- Linux permissions are usually session/backend constraints rather than central
  OS privacy prompts.

### Windows 11

- Tendril expects a normal desktop user session.
- Discovery, capture, and input do not require a Tendril-managed background
  service.
- Microphone paths may depend on **Settings > Privacy & security > Microphone**
  for desktop apps.

For a source-backed inventory of the current runtime subprocess/tool surface and
its self-containment classification, see
[`docs/src/reference/runtime-dependencies.md`](docs/src/reference/runtime-dependencies.md).

## Audio capture status

For v0.0.1, `tendril listen` ships a probe-first slice:

- it accepts explicit `--source`, `--duration-ms`, and `--format` settings,
- it returns machine-readable capability and permission diagnostics for
  loopback/system and microphone paths where the current adapter can probe
  them,
- it distinguishes unsupported capability/permission failures from transient
  platform adapter failures, and
- it explicitly reports that audio artifact emission is not implemented yet.

Documented gap for v0.0.1: explicit `device:<id>` binding is accepted by the
command surface so callers can express intent, but it returns a structured
unsupported-capability result until adapter-specific device enumeration/binding
lands.

## Release automation

Local pre-merge validation remains the primary fast-feedback gate:

```bash
./scripts/pre-merge.sh
```

Tendril uses SemVer. The release version comes from `[workspace.package].version`
in `Cargo.toml`, and release tags use the `v<semver>` form, for example
`v0.0.1`.

Pushing a `v*` tag starts the tag-only GitHub Actions release workflow, which
reruns pre-merge validation, builds Linux artifacts on `[self-hosted, linux]`,
builds macOS artifacts on `[self-hosted, macos]`, stages the combined asset set,
and publishes a GitHub release.

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
