# Tendril

## Overview

Tendril is a stateless Rust CLI for agent-driven desktop inspection and control across macOS, Linux, and Windows 11. Its primary purpose is to let agents discover interactive desktop targets, capture visual and optional audio state, and execute input sequences against those targets without requiring a background daemon.

The first launch target is **v0.0.1**, which must support:
- listing desktops/displays/windows that can be targeted,
- screenshot capture for windows and displays,
- mouse and keyboard input execution against a target,
- agent-friendly CLI and MCP access with structured JSON output,
- local machine defaults via a single config file.

Tendril must expose its CLI surface over MCP stdio so an agent can use either:
- direct CLI commands such as `tendril list`, `tendril capture`, and `tendril run`, or
- `tendril mcp stdio`, where command handlers are exposed as MCP tools.

Tendril is intentionally stateless apart from user-machine defaults stored in `$TENDRIL_CONFIG_DIR/config.yaml` (default: `~/.config/tendril/config.yaml`). It must not require a daemon or background service.

## Architecture

### High-level design

Tendril will be a Rust workspace with the following logical components:

1. **`tendril` binary crate**
   - Implements the end-user CLI.
   - Provides top-level commands and global flags.
   - Emits agent-friendly help when run with no arguments.

2. **`mcp-cli` library crate**
   - Lives inside this repo initially.
   - Provides generic reusable machinery for exposing CLI commands as MCP tools over stdio.
   - Provides generic `--json` support for machine-readable success/error envelopes.
   - Must be designed so it can later move to its own repository with minimal API breakage.

3. **core command/domain layer**
   - Defines typed command inputs, output models, validation rules, and serialization.
   - Keeps CLI and MCP execution paths identical.
   - Owns shared concepts such as targets, captures, input actions, aliases, permissions, and audio sources.

4. **platform adapter layer**
   - macOS adapter using the most reliable native APIs available for screen capture, accessibility/input control, and audio capture.
   - Linux adapter using the most reliable native APIs available per desktop/session stack.
   - Windows 11 adapter using the most reliable native APIs available for capture, input injection, and audio capture.
   - Adapters must hide platform-specific details behind a stable internal interface.

### Statelessness model

- No daemon.
- No persistent runtime state beyond `config.yaml` defaults.
- No background registry of active targets.
- Every command must be self-contained.
- Agents discover a fresh target set with `tendril list`, then pass target identifiers into later commands.
- If operators or agents want ergonomic reuse, `tendril alias` may emit shell helper functions, but those helpers are external shell conveniences rather than Tendril-maintained state.

### CLI surface

The initial CLI surface will include:

- `tendril list`
  - Lists desktops/displays/windows and other valid target metadata.
- `tendril capture --window <id>` or `tendril capture --display <id>`
  - Captures a screenshot of the selected target.
  - Supports `--max-width`, `--max-height`, `--format`, `--compression`.
  - Returns enough metadata for consumers to map coordinates from resized output back to source-space.
- `tendril --window <id> run <input-definition>`
  - Executes input against a target.
  - `input-definition` may be:
    - a plain string literal to be typed using the safest appropriate platform path,
    - or a DSL sequence such as `hold(ctrl),c,release(ctrl),wait(1s),send("abc"),hold(ctrl),send("v")`.
- `tendril listen ...`
  - Captures system audio (loopback) or microphone/device input when supported and permitted.
- `tendril alias ...`
  - Emits shell helper output so agents can source a pre-filled command wrapper for repeated targeting.
- `tendril mcp stdio`
  - Serves the CLI tool surface over MCP stdio.

### Command semantics

#### Targeting

- Targets include windows, displays/desktops, and audio sources where applicable.
- Target IDs must be stable for the duration of a single workflow but need not be durable across system restarts or app relaunches.
- `list` output must include the metadata required for later commands: target kind, identifier, dimensions, scaling information, process/app title where available, and whether the target is capturable/actionable.

#### Capture

- Capture commands must support window and display screenshots in v0.0.1.
- Resizing options must preserve aspect ratio unless explicitly overridden in a future version.
- Output metadata must include original dimensions, output dimensions, scale factors, and capture timestamp.
- If a capture was resized, coordinate transforms must be explicit so an agent can convert image-space clicks back into source-space clicks.

#### Input execution

- Plain string input is the simplest input form and should use the most reliable platform-appropriate path available.
- The DSL must support at least:
  - key taps,
  - `hold(<modifier>)`,
  - `release(<modifier>)`,
  - `send("...")`,
  - `wait(<duration>)`,
  - `lclick(x,y)`, `rclick(x,y)`, `mclick(x,y)`,
  - `drag(x0,y0,x1,y1)`.
- Coordinates are relative to the target’s top-left origin in source-space.
- If the agent derived coordinates from a scaled capture, Tendril must expose the scale math needed to translate them correctly.
- The runtime must inject small delays where required for reliability, or prefer native accessibility/input APIs when that is more robust.
- The implementation should avoid stealing focus where possible, but correctness and supported platform capabilities take precedence. Where focus is required, the command result must say so.

#### Audio capture

- Audio capture is in scope for the broader spec and may use loopback/system audio or microphone/input-device capture where supported.
- If a platform or session does not support a requested capture path, Tendril must fail clearly with a structured capability error.
- Audio capture must be permission-aware and explicit about source type, duration, format, and channel metadata.

#### Alias generation

- `tendril alias` should emit shell code that helps agents repeatedly target the same window/display without Tendril storing session state.
- The emitted alias/wrapper must remain transparent shell-level convenience, not hidden persistent Tendril state.

### MCP exposure model

- `tendril mcp stdio` must expose the relevant CLI commands as MCP tools.
- CLI and MCP must share one typed command model and one execution path.
- Tool schemas must be generated from the same typed definitions used for CLI argument parsing where practical.
- The generic `mcp-cli` crate must make `--json` / structured results and MCP tool exposure reusable for arbitrary future CLI projects.
- The MCP surface should expose stable, explicit tool names and structured error categories.

## Build Environment

### Rust and workspace

- Primary language: Rust.
- Use a Rust workspace if multiple crates are required (`tendril`, `mcp-cli`, and any shared support crates).
- Favor stable Rust unless a specific unstable feature is justified and approved.

### Nix flake

The repository must include a `flake.nix` that provides:
- a devShell with all build, lint, and documentation dependencies,
- package outputs for the Tendril binary,
- checks for formatting, linting, tests, and any relevant platform-agnostic validation,
- reproducible local and CI builds.

### direnv

- `.envrc` must use `use flake`.
- Entering the repo should prepare a correct development environment for humans and agents.

### Cacophony project config

The repo must include `.cacophony/config.yaml` with:
- project metadata,
- checkout bootstrap steps such as `direnv allow`,
- build and test command definitions,
- agent-friendly defaults for this project.

### Tendril config

- Tendril runtime defaults live in a single file at `$TENDRIL_CONFIG_DIR/config.yaml`.
- Default config directory is `~/.config/tendril`.
- The initial config schema should cover defaults such as image compression and output format.
- Runtime config must not become a hidden session store.

### CI/CD pipeline

- Use GitHub Actions on **tag pushes only**.
- Prefer self-hosted runners aligned with the Nix environment.
- CI must perform reproducible release builds, tests, and artifact publication steps appropriate for tagged releases.
- Local pre-merge hooks remain the primary gate for fast feedback before merge.

## Features

### Feature 1: Target discovery

Provide a command that enumerates valid automation and capture targets.

#### Required behavior
- List windows and displays in v0.0.1.
- Include target IDs, human-readable names/titles where available, bounds, scale information, and target type.
- Mark whether a target supports capture, input, or both.
- Support `--json` output with a stable schema.

#### Acceptance criteria
- `tendril list --json` returns structured output with at least one target kind field and one stable identifier field.
- On systems with multiple displays or windows, the result distinguishes them unambiguously.
- If platform permissions are missing, output is a structured permission/capability error rather than an opaque failure.

### Feature 2: Window/display screenshot capture

Provide a command to capture screenshots for windows and displays.

#### Required behavior
- Capture by `--window <id>` or `--display <id>`.
- Support `--max-width`, `--max-height`, `--format`, `--compression`.
- Return metadata needed for coordinate remapping after resize.
- Support machine-readable responses under `--json`.

#### Acceptance criteria
- Captures succeed for both a display target and a window target on supported platforms.
- When resize flags are applied, the response includes original dimensions, output dimensions, and scale mapping values.
- Invalid combinations of target flags are rejected with structured validation errors.
- The same command model is invocable through MCP stdio.

### Feature 3: Input automation

Provide target-scoped command execution for text entry and explicit input sequences.

#### Required behavior
- Support plain string typing for common agent use.
- Support the DSL sequence form for precise control.
- Support mouse clicks and drag actions with source-space coordinates.
- Insert or honor reliability delays as required.
- Prefer accessibility/native automation APIs when appropriate.

#### Acceptance criteria
- The DSL parser accepts and validates the initial action set defined in this spec.
- Coordinate actions behave correctly when derived from scaled screenshots using returned mapping metadata.
- Execution errors identify the failing action index and reason where possible.
- Input execution either avoids focus stealing or clearly reports when focus transfer was necessary.

### Feature 4: MCP stdio server

Expose Tendril’s command surface as MCP tools.

#### Required behavior
- `tendril mcp stdio` starts an MCP server over stdio.
- Initial MCP tool set includes discovery, capture, and input execution.
- MCP and CLI use one shared typed command model.
- Generic support should live in the in-repo `mcp-cli` crate.

#### Acceptance criteria
- MCP tool schemas match the effective CLI input model for the same operations.
- A parity test suite verifies that CLI invocation and MCP invocation return equivalent structured success/error payloads.
- The `mcp-cli` crate can be used by a second sample command surface without hard-coding Tendril-specific types.

### Feature 5: Audio capture

Provide audio inspection support for system loopback and/or input devices when supported.

#### Required behavior
- Support explicit selection of loopback/system or microphone/input device capture where platform APIs permit.
- Return structured capability/permission errors on unsupported paths.
- Support machine-readable output describing capture source and metadata.

#### Acceptance criteria
- Capability probing distinguishes unsupported platform/session combinations from transient runtime failures.
- The command surface clearly identifies source type, format, and duration settings.
- This feature may ship after v0.0.1 if platform parity or permissions make it unsuitable for the first release, but it must be represented in the implementation plan.

### Feature 6: Alias generation

Provide shell-level helpers for repeated targeting without introducing Tendril-managed session state.

#### Required behavior
- Emit shell-compatible output that can be sourced.
- Allow agents to bind a chosen target into a reusable wrapper.
- Keep the behavior transparent and inspectable.

#### Acceptance criteria
- Generated output is shell-usable and does not depend on hidden Tendril state.
- Alias output can pre-fill target selection while leaving other flags controllable.
- The command supports `--json` metadata alongside raw shell output where appropriate.

### Feature 7: Agent-friendly UX

Make Tendril easy for agents to discover and use correctly.

#### Required behavior
- Running `tendril` with no arguments prints concise, agent-oriented help.
- All primary commands support `--json` via generic `mcp-cli` functionality.
- Errors are categorized and actionable.

#### Acceptance criteria
- Help output describes the recommended workflow: list → capture → run.
- JSON output is stable enough for agents to parse without scraping human text.
- Human-readable and JSON modes remain behaviorally equivalent.

## Testing Strategy

### Framework and approach

Testing is mandatory and must be strong enough that multiple agents can build safely without repeatedly breaking each other.

The strategy will combine:
- unit tests for typed command models, validation, config parsing, and DSL parsing,
- property tests for coordinate transform math and DSL round-tripping where appropriate,
- integration tests for CLI invocation, MCP stdio parity, and target-independent execution flows,
- platform adapter contract tests using mocks/fakes where real desktop APIs are unavailable in CI,
- platform-specific smoke coverage on supported self-hosted runners where permissions and capture/input APIs can be exercised.

### Required test areas

1. **CLI validation tests**
   - flag combinations,
   - target selection rules,
   - config default resolution,
   - JSON envelope structure.

2. **DSL parser/executor tests**
   - parsing valid sequences,
   - rejecting invalid syntax with precise diagnostics,
   - preserving action order and timing semantics.

3. **Coordinate mapping tests**
   - source-space to output-space transform correctness,
   - resized capture remapping for click/drag actions,
   - edge cases near bounds and on fractional scale factors.

4. **MCP parity tests**
   - CLI and MCP results must be semantically equivalent for the same operation.

5. **Platform abstraction tests**
   - adapter capabilities,
   - permission probes,
   - unsupported-path handling.

6. **Integration/smoke tests**
   - target listing,
   - screenshot capture,
   - text entry / input sequence execution on controlled test targets where feasible.

### Resource isolation requirements

- Integration tests must use unique temp directories.
- Any ports used by tests must be allocated dynamically.
- Test artifacts must be namespaced per run.
- Tests must avoid global mutable state and must not rely on a shared daemon.
- Platform smoke tests must clearly separate destructive from non-destructive actions.

### Coverage targets

- At least **85% line coverage** in the command/domain and `mcp-cli` logic that is platform-agnostic.
- At least **80% branch coverage** for the input DSL parser and validation layer.
- Platform adapters may use lower direct coverage if constrained by OS APIs, but every adapter must satisfy the common contract test suite.

### Integration strategy

- Use a controlled test harness for target-independent flows.
- Use real platform APIs for smoke tests on self-hosted runners where allowed.
- Verify that list → capture → run works end-to-end for at least one supported target per enabled platform lane.
- Audio tests should include capability probing and format validation even if full loopback capture is not available in all CI environments.

### CI test pipeline

- Pre-merge local hooks run fast checks: formatting, linting, unit tests, and key integration tests.
- Tag-triggered GitHub Actions on self-hosted runners run the fuller release matrix.
- Nix checks must remain authoritative for reproducibility.

## Error Handling

- Use structured errors with stable categories.
- For Rust defaults:
  - use `thiserror` for library/domain error types,
  - use `anyhow` only at binary boundaries if needed.
- Error categories should include at minimum:
  - validation,
  - unsupported capability,
  - missing permission,
  - target not found,
  - platform adapter failure,
  - execution/action failure,
  - config error,
  - serialization/MCP exposure error.
- JSON mode and MCP mode must both return structured error payloads.
- Input execution errors should point to the failing action or stage where feasible.

## Logging and Observability

- Use structured logging, defaulting to `tracing` in Rust.
- Support log levels appropriate for normal use, debugging, and platform-diagnostics work.
- Logs must not require scraping human text out of command output when `--json` is in use.
- Sensitive values should be redacted when logging input payloads, device metadata, or future secrets.
- MCP server logs must avoid corrupting stdio protocol traffic; diagnostic output must be routed safely.

## Versioning and Releases

- Use SemVer.
- Initial target release is **v0.0.1**.
- Any change to stable CLI flags, JSON schemas, or MCP tool contracts is semver-relevant.
- GitHub Actions release flow runs on tag pushes only.
- Releases should produce versioned artifacts for supported platforms where feasible.
- Maintain `CHANGELOG.md` using Keep a Changelog format.
- Version strings must be updated in Rust manifests and any release metadata.

## CI/CD and Pre-merge Gates

- Fast local pre-merge hooks are mandatory and must prevent broken merges.
- For Rust + Nix, pre-merge should include at minimum:
  - format check,
  - clippy/lint,
  - unit tests,
  - selected integration/parity tests,
  - `nix flake check` or equivalent scoped validation.
- GitHub Actions handles tagged release verification and packaging on self-hosted runners.

## Protocol Versioning

- MCP tool names, argument schemas, and result schemas must be treated as versioned interfaces.
- Any future non-backward-compatible protocol change must be documented in the changelog and reflected in semver.
- If Tendril introduces any custom wire formats beyond MCP/JSON, they must be versioned from day one.

## Security

- Validate all user- and agent-supplied target IDs, coordinates, durations, formats, compression values, and device selectors.
- Keep privilege boundaries explicit: Tendril should request only the permissions needed for capture/input/audio features.
- Permission diagnostics must be clear and actionable per platform.
- Tendril should not store secrets in config.
- Commands that may affect user focus or inject input into a live target must be explicit and auditable.
- Unsafe or unsupported operations must fail closed with structured diagnostics.

## Non-functional Requirements

- **Statelessness:** no daemon, no hidden runtime store.
- **Cross-platform support:** macOS, Linux, Windows 11 through adapter isolation.
- **Reliability:** command results must be deterministic enough for agents to automate against.
- **Agent usability:** all primary flows must be scriptable via JSON or MCP without scraping text.
- **Minimal user disruption:** avoid focus stealing where possible; report when it cannot be avoided.
- **Documentation:** publish project documentation via GitHub Pages, preferably from a Rust-friendly documentation setup such as mdBook or a static docs site.

## Documentation Plan

The repository should eventually include:
- quickstart setup for permissions and platform prerequisites,
- CLI usage guide,
- MCP usage guide,
- DSL reference,
- capture coordinate/remapping explanation,
- platform support and limitations matrix.

## License

MIT
