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
logging:
  level: info
```

## Current fields

### `capture`

- `format`: `png` or `jpeg`
- `compression`: integer from `0` to `100`
- `max_width`: optional positive integer
- `max_height`: optional positive integer

### `logging`

- `level`: `error`, `warn`, `info`, `debug`, or `trace`

## Behavior

- Missing config files fall back to built-in defaults.
- Unknown fields are rejected.
- Invalid values return structured config errors.

That keeps runtime defaults explicit without turning Tendril into a stateful session manager.
