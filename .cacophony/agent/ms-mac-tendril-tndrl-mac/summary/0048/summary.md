# bd-85da62 — read-only `tendril permissions` command

Adds `tendril permissions`, a read-only CLI command that reports per-platform
input/capture permission status (Screen Recording, Accessibility, Microphone)
in one shot, with state (granted/denied/unknown/not-required) + remediation.

## Changes
- `cli.rs`: new `Command::Permissions(PermissionsCommand)` variant + `name()` arm + struct.
- `commands/mod.rs`: `dispatch_permissions_command` aggregates `adapter.permissions()`
  (the `PermissionAdapter` trait, implemented by every platform adapter) into a
  `PermissionsReport { adapter, permissions }`, rendered as JSON envelope or human
  text. Android arm = unsupported (consistent with listen/clipboard). 4 unit tests.
- `feedback.rs`: fixed 4 pre-existing `clippy --all-targets` `field_reassign_with_default`
  lints in test code so the crate is fully clippy-clean under `-D warnings`.
- `docs/src/cli/index.md`: command-map entry.

## Design
- Reuses the existing permission model (`PermissionStatus`/`FeatureSupport`/consent
  probes); no new probing logic. CLI-only (no MCP tool) for the first cut, matching
  the `alias`/`version` precedent — zero MCP fixture churn.
- Whole-crate `cargo fmt` churn on 10 unrelated files was reverted to keep the
  change focused.

## Validation
- `cargo check -p tendril` OK.
- `cargo test -p tendril --lib` = 328 passed, 0 failed.
- `cargo clippy -p tendril --all-targets -- -D warnings` clean.

## Follow-ups (not in scope)
- `tendril permissions request` (fire TCC prompt + open Settings pane).
- Optional MCP tool exposure.
