# Session summary — non-blocking webhook feedback default (bd-13c534)

## Goal

Make Tendril's env-driven breakage feedback safe-by-default: a synchronous
webhook POST on every CLI error-exit (feedback-cli's default) would add latency
and could hang the exit on a slow/unreachable endpoint. Default it to
best-effort background (non-blocking) delivery for this high-frequency stateless
CLI, while respecting any explicit project-config choice.

## Bead(s)

- `bd-13c534` — feedback-cli: prefer non-blocking webhook delivery so breakage reporting never stalls the CLI exit (self-filed reflect-session follow-up, implemented per operator "continue toward goals")
- Builds on bd-505754 / bd-26575f (feedback-cli adoption + project-config).

## Before state

- Failing tests: none (323 lib green at 5ebfb98).
- Env-driven webhook feedback (FEEDBACK_WEBHOOK_URL) inherited feedback-cli's
  `blocking: true` default => synchronous POST on every error-exit.

## After state

- Failing tests: none — cargo check + clippy clean, cargo test -p tendril --lib
  324 passed (+1 non-blocking test).
- feedback_config() demotes the env-driven webhook strategy to `blocking: false`
  (best-effort background). Explicit `[feedback]` project config is respected
  verbatim (set `blocking: true` for synchronous).

## Diff summary

- Code/content commit: c03e98f; final landed squash SHA from the reintegration receipt.
- Files touched: crates/tendril/src/feedback.rs, docs/src/reference/configuration.md.
- Tests: +1 / -0 / flipped 0 (env_webhook_defaults_to_non_blocking).
- Behavioural delta: env webhook feedback no longer blocks the CLI error-exit path.

## Operator-takeaway

Tendril's breakage->bead feedback over a webhook is now non-blocking by default
when configured via env, so reporting can never add latency or hang the CLI's
exit — important for a stateless CLI agents invoke constantly. Projects that
need synchronous delivery opt in with `blocking: true` in the `[feedback]`
config block.
