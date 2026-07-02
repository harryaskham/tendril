# Configuration

Tendril reads machine-local defaults from one file:

- environment override: `$TENDRIL_CONFIG_DIR/config.yaml`
- default path: `~/.config/tendril/config.yaml`

## Current schema

```yaml
capture:
  format: png
  compression: 85
  max_width: null
  max_height: null
  timeout_ms: null
logging:
  level: info
execution_lock:
  enabled: true
  timeout_ms: 60000
  stale_ms: 30000
  path: null
# Optional: route Tendril breakages back to the owning project (feedback-cli).
# Omit entirely to keep feedback off (the default).
feedback:
  enabled: true
  component: tendril
  strategy:
    type: webhook        # webhook | caco_cli | file | stderr | disabled
    url: https://example.invalid/feedback
    token_env: TENDRIL_FEEDBACK_TOKEN
```

## Current fields

### `capture`

- `format`: `png` or `jpeg`
- `compression`: integer from `0` to `100`
- `max_width`: optional positive integer
- `max_height`: optional positive integer
- `timeout_ms`: optional positive integer deadline for capture backends

### `logging`

- `level`: `error`, `warn`, `info`, `debug`, or `trace`

### `execution_lock`

- `enabled`: whether `tendril run` uses the default host-local execution lock/queue
- `timeout_ms`: positive integer queue wait timeout before `run` returns a structured timeout
- `stale_ms`: positive integer heartbeat age after which abandoned locks/tickets are reaped
- `path`: optional lock root override; leave `null` to use the default temp-dir user/session namespace

See [Execution lock and queue](execution-lock.md) for CLI and environment overrides.

### `feedback`

Optional breakage/feedback reporting via [`feedback-cli`](https://github.com/harryaskham/feedback-cli). When present, Tendril forwards every CLI/MCP breakage (the structured error that reaches its error sink) so the owning project can turn it into a bead / logged error. Omit the whole `feedback` block to keep it off.

- `enabled`: master on/off switch (default `true` when the block is present)
- `component`: source label on events (defaults to `tendril`)
- `project`: optional default project label on events
- `strategy.type`: `webhook` (POST to a caco feedback endpoint that files a bead), `caco_cli` (shell out to `caco log error` / file a bead), `file` (append JSON lines), `stderr`, or `disabled`
- webhook strategy: `url`, optional `token_env` (env var holding the bearer token) / `token`, optional `headers`, and `blocking: false` for best-effort background delivery

When no `feedback` block is configured, Tendril falls back to the `FEEDBACK_WEBHOOK_URL` environment variable (and is otherwise silent). As a further fallback, when `FEEDBACK_WEBHOOK_URL` is unset but a shared `FEEDBACK_WEBHOOK_BASE_URL` is set (the operator's canonical `…/hooks/global` namespace), Tendril appends its own component path and routes to `<base>/tendril` so feedback stays in its own hook namespace instead of the shared root (bd-42a4d9). An explicit config block takes precedence over the environment, and an explicit `FEEDBACK_WEBHOOK_URL` takes precedence over `FEEDBACK_WEBHOOK_BASE_URL`. The env-driven webhook fallback defaults to **non-blocking** (best-effort background) delivery so breakage reporting never adds a synchronous HTTP round-trip to the CLI's error-exit path; configure `[feedback]` with `blocking: true` if you need synchronous delivery.

## Behavior

- Missing config files fall back to built-in defaults.
- Unknown fields are rejected.
- Invalid values return structured config errors.

That keeps runtime defaults explicit without turning Tendril into a stateful session manager.
