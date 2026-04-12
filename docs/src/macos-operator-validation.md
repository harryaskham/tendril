# macOS operator validation

Use this page when you want to verify Tendril on a real macOS machine without reading the Rust code.

All examples assume:

- you are on macOS in a normal logged-in GUI session,
- you are running from the repository root, and
- Nix is installed so `nix run .#tendril -- ...` works.

If you only want the shortest possible smoke check, run `list`, then `capture`, then `run`, then the MCP example below.

## Minimal smoke-check commands

### 1. List targets

Run both the human-readable and JSON forms once:

```bash
nix run .#tendril -- list
nix run .#tendril -- list --json
```

Expected success:

- Tendril prints at least one display or window target.
- In JSON mode, the top-level status is `success` and `data.targets` is non-empty.

Representative success shape:

```json
{
  "status": "success",
  "meta": { "command": "list" },
  "data": {
    "permissions": [
      { "permission": "screen_capture" }
    ],
    "targets": [
      { "kind": "display", "id": "display-1" }
    ]
  }
}
```

Expected permission-guided failure when Screen Recording has not been granted yet:

```json
{
  "status": "error",
  "meta": { "command": "list" },
  "error": {
    "category": "missing_permission",
    "code": "missing_permission",
    "message": "macOS target discovery needs Screen Recording consent to enumerate visible windows.",
    "details": {
      "permission": "screen_capture",
      "suggested_action": "Grant Screen Recording to the invoking terminal or tendril binary, then rerun tendril list."
    }
  }
}
```

### 2. Capture a display or window

First, copy a `display` or `window` id from `nix run .#tendril -- list --json`.

Display capture example:

```bash
nix run .#tendril -- --display <display-id> capture --json --max-width 1440 > /tmp/tendril-capture.json
```

Window capture example:

```bash
nix run .#tendril -- --window <window-id> capture --json > /tmp/tendril-window-capture.json
```

Expected success:

- the command exits successfully,
- the JSON file contains `"status": "success"`, and
- the payload includes target metadata, output dimensions, and a base64 image payload.

Expected permission-guided failure when Screen Recording is still blocked:

```json
{
  "status": "error",
  "meta": { "command": "capture" },
  "error": {
    "category": "missing_permission",
    "code": "missing_permission",
    "message": "missing permission: Capture execution is gated on explicit Screen Recording consent.",
    "details": {
      "permission": "screen_capture",
      "suggested_action": "Grant Screen Recording access before invoking capture commands."
    }
  }
}
```

### 3. Run input against a window

Open a safe target first, for example TextEdit with a blank document and a visible text caret.
Then copy that window id from `list` output and run:

```bash
nix run .#tendril -- --window <window-id> run --json 'send("hello from Tendril on macOS")'
```

You can also verify the input DSL path:

```bash
nix run .#tendril -- --window <window-id> run --json 'send("hello"),wait(250ms),send(" world")'
```

Expected success:

- the target app receives the text, and
- JSON output reports `"status": "success"` with `data.action_count`.

Expected permission-guided behavior:

- macOS should ask for **Accessibility** access the first time Tendril tries to control input, or
- the invoking terminal / Tendril binary should appear under **System Settings > Privacy & Security > Accessibility**.

If input does not execute and you see a runtime/tooling error instead of Accessibility guidance, jump to [Troubleshooting self-containment failures](#troubleshooting-self-containment-failures).

### 4. Launch MCP stdio

Minimal launch:

```bash
nix run .#tendril -- mcp stdio
```

That starts the MCP server and waits for framed JSON-RPC messages on stdin.

If you want a copy-pasteable end-to-end probe that initializes the server and asks for `tools/list`, use:

```bash
frame() {
  body="$1"
  printf 'Content-Length: %s\r\n\r\n%s' "$(printf %s "$body" | wc -c | tr -d ' ')" "$body"
}

{
  frame '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
  frame '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
  frame '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
} | nix run .#tendril -- mcp stdio
```

Expected success:

- the output contains framed JSON-RPC responses,
- the first response includes `"serverInfo":{"name":"tendril"`, and
- the `tools/list` response includes the `list`, `capture`, and `run` tools.

## What macOS permission prompts/settings should appear

### Screen Recording

You should expect **Screen Recording** involvement for `list` and `capture`.

Typical operator-visible flow:

1. Run `nix run .#tendril -- list` or a capture command.
2. macOS may show a Screen Recording prompt for the invoking app.
3. The invoking app should then appear in **System Settings > Privacy & Security > Screen Recording**.
4. After allowing access, quit and reopen the terminal app if macOS asks for it, then rerun the command.

Depending on how you launch Tendril, the entry may appear as your terminal app (for example Terminal, iTerm, or WezTerm) or as the Tendril binary itself.

### Accessibility

You should expect **Accessibility** involvement for `run`.

Typical operator-visible flow:

1. Open a harmless target such as TextEdit.
2. Run a `tendril run` command.
3. macOS may show an Accessibility prompt for the invoking app.
4. The invoking app should appear in **System Settings > Privacy & Security > Accessibility**.
5. Enable it, then rerun the command.

If the prompt does not appear automatically, check the Accessibility settings pane directly.

### Microphone

This page focuses on `list`, `capture`, `run`, and MCP stdio, but if you later validate `listen --source microphone`, macOS should use **System Settings > Privacy & Security > Microphone** for that consent boundary.

## Troubleshooting self-containment failures

These are not the desired long-term operator experience, but they are useful for diagnosing current macOS validation failures.

### `swift` is missing

Symptoms may look like:

- `platform_adapter_failure` during `list`,
- `execution_failure` or `input_spawn_failed` during `run`, or
- stderr text such as `error: tool 'swift' not found`.

What it means:

- the current macOS adapter still shells out to the Swift toolchain for some discovery and input paths,
- so the binary is not yet fully self-contained on macOS, and
- permission guidance may be masked by the missing runtime dependency.

Current tracking bug: `bd-5c3937`.

Temporary workaround if you need to continue local validation today:

```bash
xcode-select --install
```

Then rerun the Tendril command after the Command Line Tools install completes.

### Permissions were granted but the command still fails

Try this sequence:

1. confirm the app you used to launch Tendril is enabled in the relevant privacy pane,
2. fully quit and reopen that app,
3. rerun `nix run .#tendril -- list --json`, and then
4. retry `capture` or `run`.

### MCP stdio appears to hang

That usually means Tendril is waiting for properly framed MCP input.

- `nix run .#tendril -- mcp stdio` is supposed to wait on stdin.
- Use an MCP client, or use the `frame()` shell helper above so the request includes `Content-Length` headers.
