# `tendril run`

Use `tendril run` to send text or explicit input sequences to a selected target.

## Example

```bash
tendril --window <id> run 'send("hello")'
tendril --window <id> run 'hold(ctrl),c,release(ctrl),wait(250ms),send("done")'
tendril --window <id> run --no-restore-focus 'send("leave focus here")'
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

## Focus restoration

`tendril run` restores prior user focus by default when the active adapter can observe and restore it. This keeps short automation bursts from leaving the operator's desktop focused on the agent-targeted window.

- Use the default `--restore-focus` behavior for quick agent actions on a shared desktop.
- Use `--no-restore-focus` when a workflow intentionally wants focus to remain on the target, for example before a longer manual/operator handoff.
- On Linux X11, Tendril records the active window and pointer position before target-scoped input, then restores both after a successful run.
- On generic Wayland, macOS, and Windows sessions where Tendril cannot yet perform a reliable restore in this adapter path, the command still succeeds but reports `focus_restored: false` and a clear `restore_error`/note explaining the limitation.

Agents should still follow the capture-act-verify loop and avoid racing the operator. If a run reports restoration failure, pause and recapture before sending more input.

## Result details

Structured output reports:

- the selected target,
- whether focus was required,
- whether focus was transferred,
- the number of executed actions,
- `previous_focus` when a pre-run focus snapshot was captured,
- whether focus was restored,
- whether the pointer was restored,
- `restore_error` when restoration was requested but unavailable or failed, and
- any adapter notes.

When execution fails, the error payload can include the failing action index.
