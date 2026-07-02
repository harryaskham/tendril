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
      { "kind": "display", "id": "1" }
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

If input does not execute and you see a runtime/tooling error instead of Accessibility guidance, jump to [Troubleshooting](#troubleshooting).

### 4. Launch MCP stdio

Minimal launch:

```bash
nix run .#tendril -- mcp stdio
```

That starts the MCP server and waits for newline-delimited JSON-RPC messages on stdin (one compact JSON object per line).

If you want a copy-pasteable end-to-end probe that initializes the server and asks for `tools/list`, use:

```bash
frame() {
  body="$1"
  # MCP stdio framing is newline-delimited JSON: one compact JSON object per line.
  printf '%s\n' "$body"
}

{
  frame '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
  frame '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
  frame '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
} | nix run .#tendril -- mcp stdio
```

Expected success:

- the output contains newline-delimited JSON-RPC responses,
- the first response includes `"serverInfo":{"name":"tendril"`, and
- the `tools/list` response includes the `list`, `capture`, and `run` tools.

## What macOS permission prompts/settings should appear

### Requesting permissions programmatically (`tendril permissions --request`)

By default `tendril permissions` is a read-only probe. In a foreground/operator
session you can opt into surfacing the real OS prompts instead of navigating
System Settings by hand:

```
tendril permissions --request --json
```

On macOS this:

- runs the `screencapture` registration probe (so Tendril appears under Screen
  Recording and the first-run prompt can be answered),
- surfaces the **Accessibility** prompt via `AXIsProcessTrustedWithOptions`
  (through the same `osascript` JXA ObjC bridge Tendril already uses for element
  discovery — no Rust FFI, so the workspace stays `unsafe_code = "forbid"`), and
- opens the matching **System Settings > Privacy & Security** panes for Screen
  Recording, Accessibility, and Microphone.

The JSON envelope's `data.requested[]` array reports, per permission, the
actions attempted, the re-probed `state_after`, and an attribution `note`.

**Attribution / persistence caveat.** macOS binds every TCC grant to the
*responsible* process. Because Tendril performs capture/input by shelling out to
stable system binaries (`screencapture`, `osascript`), the grants attach to
those helpers and/or the parent launcher (your terminal, `sshd`, or the caco
daemon) rather than to the Tendril binary's nix-store path. This is what makes
grants **persist across Tendril nix-store updates** — you grant the stable
system helper once, not the versioned binary every release. For a grant bound to
Tendril's own signed identity (durable across every invocation path, including a
future in-process FFI backend), see the signed `.app` bundle work in bd-5110d9.

`--request` is never auto-fired and never prompts for headless/daemon callers
(the majority of fleet usage): auto-prompting would block unattended runs on an
unanswerable GUI dialog. Set `TENDRIL_SKIP_PERMISSION_PROBE=1` to report the
plan without performing any side effects.

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

## Troubleshooting

### Validating over `caco ssh ms-mac` / SSH

When validating from another machine through `caco ssh ms-mac` or plain SSH,
prefer the already-installed packaged `tendril` binary for runtime smoke checks:

```bash
caco ssh ms-mac -- 'tendril --json list'
caco ssh ms-mac -- 'tendril --display 1 capture --json --timeout-ms 2000'
```

Avoid adding shell pipelines that implicitly use the remote Nix profile's
`coreutils` (for example `| head`) while diagnosing Tendril itself. If you need
small output slices, use an OS tool with an absolute path such as `/usr/bin/head`
or redirect to a file and inspect it separately.

A known host/toolchain failure mode is a `dyld` message like:

```text
Library not loaded: /nix/store/.../libcurl.4.dylib
code signature ... not valid for use in process: library load mig callout failed
```

or the same shape for `libgmp` / `librustc_driver`. That is a macOS code-signing
problem in the remote Nix/rustup toolchain or helper process, not a Tendril
runtime failure. In that state:

- do not treat `cargo`, `rustc`, or Nix helper failures over SSH as evidence that
  the Tendril binary is broken;
- validate packaged runtime behavior with `tendril ...` directly;
- if source builds are required on the Mac, refresh/repair the Mac toolchain or
  use the self-hosted macOS runner path rather than ad-hoc SSH builds; and
- keep the full `dyld` line in any bug report so the failing library and launcher
  are visible.

### Permissions were granted but the command still fails

Try this sequence:

1. confirm the app you used to launch Tendril is enabled in the relevant privacy pane,
2. fully quit and reopen that app,
3. rerun `nix run .#tendril -- list --json`, and then
4. retry `capture` or `run`.

### MCP stdio appears to hang

That usually means Tendril is waiting for properly framed MCP input.

- `nix run .#tendril -- mcp stdio` is supposed to wait on stdin.
- Use an MCP client, or use the `frame()` shell helper above so each request is sent as one newline-terminated JSON line.
