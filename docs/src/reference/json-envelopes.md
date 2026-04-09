# JSON envelopes

When `--json` is enabled, Tendril emits stable success and error envelopes shared with the MCP tool surface.

## Success shape

```json
{
  "status": "success",
  "meta": {
    "schema_version": 1,
    "command": "list"
  },
  "data": {}
}
```

## Error shape

```json
{
  "status": "error",
  "meta": {
    "schema_version": 1,
    "command": "capture"
  },
  "error": {
    "category": "validation",
    "code": "invalid_capture_input",
    "message": "validation error: max_width must be greater than zero",
    "details": {
      "field": "max_width"
    }
  }
}
```

## Stable categories

The shared `mcp-cli` layer currently uses these error categories:

- `validation`
- `unsupported_capability`
- `missing_permission`
- `target_not_found`
- `platform_adapter_failure`
- `execution_failure`
- `config_error`
- `serialization_error`

## Why this matters for publishing

The docs site includes this contract because it is part of Tendril's public interface just as much as command names or Rust APIs.
