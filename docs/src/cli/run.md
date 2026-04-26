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

### Text vs DSL ambiguity guard

A single argument without DSL metacharacters is normally treated as plain text. To avoid accidentally typing command-looking input into the focused window, Tendril rejects single bare segments that look like DSL mistakes instead of silently demoting them to text.

Examples that now fail with `invalid_run_input` and a structured `hint`:

```bash
tendril --window <id> run 'Return'
tendril --window <id> run 'type "hi"'
```

Use explicit DSL syntax for automation text, such as `send("hi")` or `send("hi"),Return`. Genuine text remains valid, for example `tendril --window <id> run 'hello world'` still types `hello world`.

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

## Host-local execution lock

`tendril run` uses a host-local execution lock/queue by default so concurrent agents do not interleave keystrokes, clicks, focus changes, or waits on the same desktop session.

Useful controls:

```bash
# default: wait in the local queue
tendril --window <id> run 'send("hello")'

# opt out only if another layer already serializes desktop control
tendril --window <id> run --no-lock 'send("hello")'

# bound queue waiting
tendril --window <id> run --lock-timeout-ms 5000 'send("hello")'

# isolate a test/sandbox lock root
tendril --window <id> run --lock-path /tmp/my-tendril-lock 'send("hello")'
```

See [Execution lock and queue](../reference/execution-lock.md) for JSON metadata, stale-lock behavior, config, and environment controls.

## Result details

Structured output reports:

- the selected target,
- whether focus was required,
- whether focus was transferred,
- the number of executed actions,
- `previous_focus` when a pre-run focus snapshot was captured,
- whether focus was restored,
- whether the pointer was restored,
- `restore_error` when restoration was requested but unavailable or failed,
- execution-lock/queue metadata, and
- any adapter notes.

When execution fails, the error payload can include the failing action index. If waiting for the execution lock times out, the error payload includes `execution_lock`, `holder`, and queue-depth details.
