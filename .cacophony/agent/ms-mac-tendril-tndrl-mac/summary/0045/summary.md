# Session summary — macOS scroll/double-click input + feedback-cli adoption

## Goal

Two pieces of Tendril work driven to a landable state in one crash-recovered
session: (1) implement native macOS scroll + double-click input injection so
`scroll(...)`/`dblclick(...)` post real OS events (recovered from an
uncommitted crash and preserved); and (2) adopt the shared `feedback-cli` so
Tendril breakages can be routed back to the owning project as beads, confirming
`updatable-cli` (the self-update half of the stack) was already fully wired.

## Bead(s)

- `bd-76e13a` — [macos-input] Implement native macOS wheel injection (scroll/CGEvent) in the Tendril macOS adapter
- feedback-cli adoption (operator-requested; bead to be created/linked at reintegration) — "use updatable-cli + feedback-cli for in-CLI breakage feedback to beads"
- Cross-refs (cacophony): bd-f3f8c7, bd-f8c259, bd-5c3937 (macOS QA / scroll); CLI stack: mcp-cli, updatable-cli, feedback-cli, omni-cli (reference consumer)

## Before state

- Failing tests: none (320 tendril lib tests green at base ca54248).
- macOS adapter returned `unsupported_scroll_action` / `unsupported_double_click_action`.
- `updatable-cli` was a dependency and wired (updater_config + register_update_tool
  + maybe_apply_staged_update); `feedback-cli` was entirely absent.

## After state

- Failing tests: none — `cargo check -p tendril` green, `cargo clippy -p tendril`
  clean, `cargo test -p tendril --lib` 322 passed / 0 failed.
- macOS `scroll(...)` posts a native `CGEventCreateScrollWheelEvent` (line units,
  sign-corrected); `dblclick(...)` posts two click-state mouse pairs.
- `feedback-cli` adopted: new `feedback.rs` routes any `TendrilError`
  (impls `mcp_cli::StructuredError`) back to the project via the configured
  strategy (webhook→caco→bead / caco-cli / file / stderr); hooked into the
  central `emit_error` sink; feedback MCP tools registered. Opt-in via
  `FEEDBACK_WEBHOOK_URL` (default Disabled = silent, no UX change).

## Diff summary

- Code/content commits: f556e07 (macOS scroll/dblclick), 0233032 (feedback-cli);
  final landed squash SHA from the reintegration receipt.
- Summary artefact commit: intentionally omitted (no self-reference).
- Files touched: crates/tendril/src/platform.rs (+ tests), docs/src/cli/run.md,
  crates/tendril/src/feedback.rs (new), crates/tendril/src/lib.rs,
  crates/tendril/src/commands/mod.rs, Cargo.toml, crates/tendril/Cargo.toml,
  Cargo.lock.
- Tests: +5 / -0 / flipped 0 (3 macOS script-builder tests + 2 feedback tests;
  MCP tool-list fixture updated for feedback_report/feedback_status).
- Behavioural delta: native macOS scroll/double-click; opt-in breakage→bead feedback.

## Operator-takeaway

The durable macOS wheel-injection fix that cacophony macOS QA was blocked on is
now native in Tendril, and Tendril now speaks the full harryaskham CLI stack:
`mcp-cli` (base) + `updatable-cli` (self-update, already wired) + `feedback-cli`
(new — breakages can be fed back to beads, opt-in via `FEEDBACK_WEBHOOK_URL`).
Watch-item: the Nix/crane build resolves `feedback-cli` as a git dependency the
same way `updatable-cli` already does; local cargo is green, but the canonical
merge-queue runner / a `nix build` should confirm crane's mcp-cli graft resolves
the new dependency (see the flake's mcp-cli graft note). This work was nearly
lost to a crash mid-daemon-outage and was saved by committing + re-validating.
