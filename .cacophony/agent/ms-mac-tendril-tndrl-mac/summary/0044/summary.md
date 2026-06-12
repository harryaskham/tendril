# Session summary — Land mcp-cli pin bump (Content-Length → NDJSON MCP transport)

## Goal

Bump Tendril's `mcp-cli` pin to current upstream main (`941015b`) and land it.
The prior session had this work build-green but HELD on my agent branch pending
an operator decision, because the bump carries a breaking MCP stdio wire-protocol
change. This session resumed after a context-loss refresh, audited the stale
claim, preserved the work, surfaced the decision, and — on the operator's
explicit "please don't hold reintegrations, we need them so agents can
collaborate" instruction — landed it with the contract docs brought into sync.

## Bead(s)

- `bd-6ffb52` — Bump mcp-cli pin to current upstream main

## Before state

- `bd-6ffb52` `in_progress`, assigned to me, last claimed 2026-06-10; session
  came up fresh (no summaries) — the lost-context-after-refresh case the
  operator warned about in a 2026-06-12 broadcast.
- Build-green work for the bump was sitting **uncommitted** in the checkout
  (risk of loss on any rebase).
- `mcp-cli` pinned at `9e2f1fc` (Content-Length / LSP-style stdio framing).
- Contract + operator-validation docs documented Content-Length framing.
- Failing tests: none observed (prior `nix build .#mcp-cli .#tendril` green with
  the staged change).

## After state

- Uncommitted work preserved as commit `1f37d60`, then rebased cleanly onto
  current `origin/main`.
- `mcp-cli` pinned at `941015b` across `flake.nix`, `flake.lock`, `Cargo.lock`.
- `crates/tendril/tests/common/mod.rs` framing helper updated Content-Length →
  newline-delimited JSON to match the new transport.
- Docs updated to NDJSON: `pi-cacophony-mcp-contract.md`, `mcp.md`,
  `macos-operator-validation.md`, `linux-wayland-operator-validation.md`.
- Failing tests: none expected; build inputs byte-identical to the prior green
  `nix build .#mcp-cli .#tendril`; merge-queue/reintegration gate is the
  authoritative validation on the merge commit.

## Diff summary

- Code/content commits: `1f37d60` (pin bump + test framing helper) plus this
  session's docs commit; final landed squash SHA comes from the reintegration
  receipt.
- Summary artefact commit: intentionally omitted (no self-reference).
- Files touched: `flake.nix`, `flake.lock`, `Cargo.lock`,
  `crates/tendril/tests/common/mod.rs`, `docs/src/reference/pi-cacophony-mcp-contract.md`,
  `docs/src/mcp.md`, `docs/src/macos-operator-validation.md`,
  `docs/src/linux-wayland-operator-validation.md`.
- Tests: framing helper rewritten (Content-Length → NDJSON); no test count delta.
- Behavioural delta: Tendril's MCP stdio transport is now newline-delimited JSON
  (one compact JSON object per line, `\n`-terminated, blank separators tolerated)
  — the MCP-spec-correct transport. This replaces the prior Content-Length (LSP)
  framing.

## Operator-takeaway

This landing flips Tendril's MCP stdio wire protocol from Content-Length headers
to newline-delimited JSON. It is MCP-spec-correct, but any Pi/Cacophony MCP
client that was built to Tendril's old Content-Length framing MUST move to NDJSON
in lockstep, or live Pi→Tendril MCP (the `tendril-mcp` tool surface many agents
use) will break. Standard MCP SDK clients already speak NDJSON. I broadcast this
cutover at landing. Follow-up not done here: the vendored `crates/mcp-cli` source
still shows the old Content-Length framing (it is grafted over by the flake input
at build time, so it does not affect builds, but it is now stale/misleading and
worth syncing).
