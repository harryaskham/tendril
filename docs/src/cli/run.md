# `tendril run`

Use `tendril run` to send text or explicit input sequences to a selected target.

## Example

```bash
tendril --window <id> run 'send("hello")'
tendril --window <id> run 'hold(ctrl),c,release(ctrl),wait(250ms),send("done")'
```

## Input forms

The current shared command model supports three payload shapes:

- plain text,
- DSL sequences, and
- normalized action lists.

The CLI accepts an `input_definition` argument and validates it into the shared typed run model used by the CLI and MCP tool surface.

## Supported action families

The current model covers:

- key taps,
- modifier hold and release,
- `send("...")`,
- timed waits,
- left/right/middle clicks, and
- drag gestures.

## Result details

Structured output reports:

- the selected target,
- whether focus was required,
- whether focus was transferred,
- the number of executed actions, and
- any adapter notes.

When execution fails, the error payload can include the failing action index.
