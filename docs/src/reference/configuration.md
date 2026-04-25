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

## Behavior

- Missing config files fall back to built-in defaults.
- Unknown fields are rejected.
- Invalid values return structured config errors.

That keeps runtime defaults explicit without turning Tendril into a stateful session manager.
