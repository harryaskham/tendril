# Session summary — Route tendril feedback to the canonical global hook base URL + /tendril (bd-42a4d9)

## Goal

Operator (Harry) set a canonical shared feedback hook: one global token + one `/hooks/global` namespace, with `FEEDBACK_WEBHOOK_BASE_URL=http://helsinki.miku-owl.ts.net:11300/hooks/global` and `FEEDBACK_WEBHOOK_TOKEN_ENV=CACOPHONY_FEEDBACK_TOKEN`, and asked each project to route its feedback to `base + /<project>` to stop cross-project feedback spam. Make tendril honor that config and route to `<base>/tendril`.

## Bead(s)

- `bd-42a4d9` — Route tendril feedback-cli to the canonical global hook base URL + /tendril path [P2, task]

## Before state

- The shared `feedback-cli` `FeedbackConfig::from_env()` only reads the FULL `FEEDBACK_WEBHOOK_URL`; it ignores `FEEDBACK_WEBHOOK_BASE_URL` and never appends a project/component path. So under Harry's canonical config (BASE_URL only, no full URL), tendril feedback would be Disabled (unrouted) — or, with a stale full URL, spam another project's namespace.

## After state

- `crates/tendril/src/feedback.rs` `feedback_config()`: when there is no `[feedback]` config and no `FEEDBACK_WEBHOOK_URL`, but `FEEDBACK_WEBHOOK_BASE_URL` is set, it builds a Webhook strategy via a new pure helper `webhook_from_base_url(base, token_env)` = `<base trimmed>/tendril` with `token_env = FEEDBACK_WEBHOOK_TOKEN_ENV`, non-blocking. Precedence preserved: `[feedback]` block > `FEEDBACK_WEBHOOK_URL` > `FEEDBACK_WEBHOOK_BASE_URL` + component. Empty/whitespace base stays Disabled.
- Unit test `webhook_from_base_url_appends_component_path` (env-free, matching the crate's no-ambient-env test philosophy). Full lib suite: 346 passed. rustfmt + `clippy --workspace --all-targets --all-features -D warnings` clean.
- Docs updated: feedback.rs module doc, `docs/src/reference/configuration.md`, `docs/src/mcp.md`.

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt (PR-backend auto-merge on green ci.yml).
- Files touched: `crates/tendril/src/feedback.rs` (helper + wiring + module doc + unit test), `docs/src/reference/configuration.md`, `docs/src/mcp.md`.
- Tests: +1 unit test; full lib suite green (346). Behavioural delta: tendril feedback now routes to `<base>/tendril` when `FEEDBACK_WEBHOOK_BASE_URL` is set and no full URL is; unchanged when the base var is absent (still opt-in / disabled).

## Operator-takeaway

Tendril now honors the fleet-canonical `FEEDBACK_WEBHOOK_BASE_URL` by appending its own `/tendril` hook path, so a single global base URL + token routes every project's feedback into its own namespace without per-project full-URL config. Same shape would apply to any other project's feedback-cli wiring (append the project's component/hook name to the shared base).
