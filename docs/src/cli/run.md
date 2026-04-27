# `tendril run`

Use `tendril run` to send text or explicit input sequences to a selected target.

## Example

```bash
tendril --window <id> run 'send("hello")'
tendril --window <id> run 'hold(ctrl),c,release(ctrl),wait(250ms),send("done")'
tendril --window <id> run 'hover(320,240),wait(250ms)'
tendril --window <id> run 'dblclick(320,240),wait(250ms)'
tendril --window <id> run 'scroll(220,420,8),wait(250ms)'
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

### Unicode text on Linux/X11

Linux/X11 key synthesis is limited by the active keyboard map: XTEST can only press keycodes that the X server maps to the requested character. ASCII text continues to use direct key events. When `send("...")` encounters text such as `Café π — emoji ✓ quoted text` that is not present in the keyboard map, Tendril falls back to a transient X11 selection owner and asks the focused application to paste the UTF-8 text.

For ordinary editable controls, Tendril temporarily owns `CLIPBOARD` and dispatches Ctrl+V. For XTerm-like terminal targets, Ctrl+V is not a paste shortcut, so Tendril tries terminal-compatible paste semantics first: `PRIMARY` + Shift+Insert, then `CLIPBOARD` + Shift+Insert, then `CLIPBOARD` + Ctrl+Shift+V before falling back to Ctrl+V. This lets agents send Unicode shell commands such as `printf '%s\n' 'Terminal 😀 Café π — ✓'` into X11 terminals.

The fallback is intentionally short-lived and visible in JSON `notes`: Tendril owns the selected X11 selection only for the paste serve window, serves the UTF-8 text to the requesting application, and releases ownership before `run` returns. If no application requests the selection data, `run` fails with `clipboard_paste_unserved` instead of silently claiming success. Release explicit held modifiers before a Unicode `send(...)`; the fallback does not run while a DSL `hold(...)` modifier is still active.

### Browser navigation guard on Linux/X11

On Linux/X11, Tendril can focus the browser's top-level window, but the browser still decides how synthetic XTEST key chords interact with its current internal focus. Firefox can keep focus inside an existing page input, so a sequence like this may report successful dispatch while typing the URL into the page instead of navigating:

```bash
# Rejected for X11 browser window targets when the text looks like a URL.
tendril --window <browser-id> run \
  'hold(ctrl),l,release(ctrl),send("file:///tmp/task.html"),Return'
```

Tendril rejects that known-unsafe browser-navigation shape with `invalid_run_input` and a remediation hint instead of silently typing URL text into the page. Use a capture-act-verify pattern that targets the visible browser chrome:

1. Capture the browser and identify the address bar coordinates in source-space.
2. Click the visible address bar.
3. Select existing address text, type the URL, and press Return.
4. Recapture and verify the page/title changed before sending page clicks.

```bash
tendril --window <browser-id> capture -o browser-before.png
# Convert capture-space to source-space if the capture was scaled.
tendril --window <browser-id> run \
  'lclick(<address_bar_x>,<address_bar_y>),hold(ctrl),a,release(ctrl),send("file:///tmp/task.html"),Return,wait(1000ms)'
tendril --window <browser-id> capture -o browser-after.png
```

This keeps normal text input and page shortcuts available while preventing the specific Ctrl+L URL-send failure mode observed on X11 Firefox.

## Supported action families

The current model covers:

- key taps,
- modifier hold and release,
- `send("...")`,
- timed waits,
- left/right/middle clicks,
- pointer-only moves/hover with `move(x,y)` or `hover(x,y)`,
- left-button double-clicks,
- drag gestures, and
- wheel scrolls with `scroll(x,y,dy)`.

`move(x,y)` and the alias `hover(x,y)` move the pointer to source-space coordinates `x,y` without pressing a mouse button. Use this for menu hover states, tooltips, map/canvas overlays, autohide UI, and other capture-act-verify workflows where a click would mutate state. Malformed pointer-move actions (wrong argument count or non-numeric coordinates) fail with structured `invalid_run_input` diagnostics naming the parse stage, action index, and offending action. The shared typed action-list model represents the same gesture as `{ "type": "pointer_move", "x": <number>, "y": <number> }`. Linux/X11 delivery uses XTEST motion events and preserves the normal focus transfer, focus restoration, pointer restoration, and execution-lock behavior used by other `run` actions; pass `--no-restore-focus` when a workflow intentionally needs the pointer to remain hovering for a later manual step or capture.

`dblclick(x,y)` and the alias `doubleclick(x,y)` move the pointer to source-space coordinates `x,y` and dispatch a calibrated primary-button double-click. Malformed double-click actions (wrong argument count or non-numeric coordinates) fail with structured `invalid_run_input` diagnostics naming the parse stage, action index, and offending action. The shared typed action-list model represents the same gesture as `{ "type": "double_click", "x": <number>, "y": <number> }`. Linux/X11 delivery uses XTEST and preserves the normal focus transfer, focus restoration, pointer restoration, and execution-lock behavior used by other `run` actions.

`scroll(x,y,dy)` moves the pointer to source-space coordinates `x,y` and sends native wheel ticks under that pointer. Positive `dy` scrolls down, negative `dy` scrolls up, and `dy=0` is rejected as `invalid_run_input`. Linux/X11 delivery uses XTEST wheel buttons and currently accepts up to 120 ticks in one action to prevent accidental unbounded event loops.

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
