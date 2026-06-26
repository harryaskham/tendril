# Session summary — feedback-cli reporting strategy from project config

## Goal

Complete the feedback-cli adoption by letting Tendril's reporting strategy be
selected from its project config (`[feedback]`), matching feedback-cli's
"configurable from project config" design, rather than only from the
`FEEDBACK_WEBHOOK_URL` environment fallback shipped in the initial adoption.

## Bead(s)

- Follow-up to the landed feedback-cli adoption (bd-505754, closed) and the
  operator request to use updatable-cli + feedback-cli for in-CLI breakage
  feedback to beads. Bead to be created/linked at reintegration.

## Before state

- Failing tests: none (322 lib + integration green at 56e9d14).
- feedback-cli was wired but strategy was env-only (FEEDBACK_WEBHOOK_URL); the
  Tendril config had no feedback field.

## After state

- Failing tests: none — cargo check + clippy clean, cargo test -p tendril green
  (323 lib + all integration tests).
- TendrilConfig has an optional `[feedback]` FeedbackConfig field; the CLI
  breakage auto-report (emit_error) and the feedback MCP tools both honour it,
  with env as the fallback and Disabled (silent) when unconfigured.

## Diff summary

- Code/content commit: db9a939; final landed squash SHA from the reintegration
  receipt.
- Summary artefact commit: intentionally omitted.
- Files touched: crates/tendril/src/config.rs, crates/tendril/src/feedback.rs,
  crates/tendril/src/lib.rs, crates/tendril/src/commands/mod.rs,
  docs/src/reference/configuration.md, docs/src/mcp.md.
- Tests: +1 / -0 / flipped 0 (project-config-precedence feedback test;
  feedback_config signature updated).
- Behavioural delta: feedback strategy is now selectable from project config,
  precedence config > env > disabled.

## Operator-takeaway

Tendril's breakage->bead feedback can now be configured declaratively in the
Tendril config file (`[feedback]` block), not just via env, completing the
feedback-cli stack integration. Default behaviour is unchanged (silent unless
configured). This builds directly on the env-driven feedback-cli adoption that
landed at 56e9d14.
