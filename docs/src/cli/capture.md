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
- `-o, --output <path>` — write the decoded image directly to a file. This
  flag is side-effecting: when combined with `--json` the JSON envelope is
  still printed to stdout while the image bytes are written to `<path>`.

## Save to file

```bash
# Save a PNG screenshot to disk (no JSON envelope on stdout)
tendril --window <id> capture -o /tmp/screen.png

# Save to disk and also emit the JSON envelope on stdout
tendril --json --window <id> capture -o /tmp/screen.png
```

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
