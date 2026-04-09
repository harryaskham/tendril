# `tendril capture`

Use `tendril capture` to request a screenshot for a selected display or window.

## Example

```bash
tendril --window <id> capture --json --max-width 1280 --format png --compression 90
```

## Current flags

- `--max-width <pixels>`
- `--max-height <pixels>`
- `--format <png|jpeg>`
- `--compression <0-100>`

## Response shape

The current JSON output includes:

- the selected target,
- original bounds and output bounds,
- explicit source-to-output and output-to-source transforms,
- whether resizing occurred,
- image encoding metadata, and
- a base64-encoded image payload.

That makes the capture response usable as an input to later coordinate-based actions.

## Config interaction

If a flag is omitted, Tendril falls back to values from `config.yaml` for capture format, compression, and optional max dimensions.

See [Configuration](../reference/configuration.md) for defaults.
