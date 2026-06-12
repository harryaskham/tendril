# Linux Wayland operator validation

Use this page when you want to validate Tendril on a real Linux Wayland desktop without reading the Rust implementation.

All examples assume:

- you are in an active Wayland login session,
- you are running from the repository root, and
- `nix run .#tendril -- ...` is available.

## Supported Wayland matrices

Tendril currently treats Wayland as a matrix of **discovery backends** plus a **capture backend**:

| Session family | Discovery backend Tendril expects | Capture backend Tendril prefers | Compatibility fallback | Generic input support |
| --- | --- | --- | --- | --- |
| Hyprland | `hyprctl` | `xdg-desktop-portal` screenshot backend (for example `xdg-desktop-portal-hyprland`) | `grim` | `ydotool` (preferred) or `wtype` (keyboard-only) — `bd-408572` |
| sway | `swaymsg` | `xdg-desktop-portal` screenshot backend (commonly `xdg-desktop-portal-wlr`) | `grim` | `ydotool` (preferred) or `wtype` (keyboard-only) — `bd-408572` |
| wlroots-compatible compositor with output enumeration only | `wlr-randr` | `xdg-desktop-portal` screenshot backend (commonly `xdg-desktop-portal-wlr`) | `grim` | `ydotool` (preferred) or `wtype` (keyboard-only) — `bd-408572` |

Important current limitation:

- `wlr-randr` only gives display discovery, not generic window discovery, so the wlroots fallback matrix is primarily a **display capture** validation path.

## Minimal validation flow

### 1. Confirm the backend tools for your compositor family

Pick the row above that matches your session and verify the matching discovery tool is available:

```bash
command -v hyprctl || command -v swaymsg || command -v wlr-randr
```

Also verify whether a portal screenshot backend is available:

```bash
systemctl --user --no-pager status xdg-desktop-portal.service
```

If the portal service is missing or unhealthy, Tendril may fall back to `grim` if that helper is installed and your compositor allows geometry-scoped screenshots.

### 2. List targets

Run both forms once:

```bash
nix run .#tendril -- list
nix run .#tendril -- list --json
```

Expected success:

- Tendril prints at least one display target.
- On Hyprland or sway, you should usually see both display and window targets.
- In JSON mode, the top-level status is `success` and `data.targets` is non-empty.

Expected structured backend error when the compositor-specific discovery tool is missing:

```json
{
  "status": "error",
  "meta": { "command": "list" },
  "error": {
    "category": "unsupported_capability",
    "code": "unsupported_capability",
    "message": "Wayland discovery requires compositor metadata from one of the supported backends: Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output enumeration (`wlr-randr`); none detected on PATH.",
    "details": {
      "capability": "target_discovery",
      "reason": "unsupported_feature",
      "suggested_action": "Use the backend that matches the active Wayland session: `hyprctl` for Hyprland, `swaymsg` for sway, or `wlr-randr` for wlroots-based display discovery. Capture no longer requires `grim` when an xdg-desktop-portal screenshot backend is available."
    }
  }
}
```

### 3. Capture a display or window

First copy a target id from `nix run .#tendril -- list --json`.

Display capture example:

```bash
nix run .#tendril -- --display <display-id> capture --json > /tmp/tendril-wayland-display.json
```

Window capture example on Hyprland or sway:

```bash
nix run .#tendril -- --window <window-id> capture --json > /tmp/tendril-wayland-window.json
```

Expected success:

- Tendril returns `"status": "success"`.
- The payload includes target metadata, output dimensions, and a base64 image payload.
- On a portal-backed session, Tendril should not require `grim` for the successful capture path.

Headless/Sunshine-style display honesty smoke (for example helsinki's simulated `sunshine-headless` output):

```bash
mkdir -p summaries/wayland-smoke
nix run .#tendril -- list --json | tee summaries/wayland-smoke/list.json
nix run .#tendril -- --display 1 capture --json --timeout-ms 2000 -o summaries/wayland-smoke/display.png
```

Expected result on a known simulated headless output:

- `list --json` succeeds, but the display target reports `capabilities.capture: false`.
- The display target includes a `diagnostics` entry with code `wayland_headless_display_capture_unavailable` and suggested remediation.
- `capture` returns quickly with `unsupported_capability` / `capture_not_supported_for_target` rather than spending the command budget on a portal timeout.
- No `display.png` is created. If a future compositor/backend makes capture work and `list` honestly reports `capture: true`, keep the resulting image under `summaries/...` rather than `/tmp` so Cacophony summaries and capture watchers can see it.

Expected structured backend error when neither a portal screenshot backend nor `grim` is available:

```json
{
  "status": "error",
  "meta": { "command": "capture" },
  "error": {
    "category": "unsupported_capability",
    "code": "unsupported_capability",
    "message": "Wayland capture needs either an xdg-desktop-portal screenshot backend or the `grim` compatibility fallback for this session; portal capture failed: ...",
    "details": {
      "capability": "window_capture",
      "reason": "unsupported_feature",
      "suggested_action": "Install and run an xdg-desktop-portal screenshot backend for the active compositor, or install `grim` as a compatibility fallback if that compositor permits geometry-scoped screenshots."
    }
  }
}
```

### 4. Verify Wayland input injection (`bd-408572`)

Wayland input now works on Hyprland and other wlroots compositors when either `ydotool` (preferred, full keyboard + pointer) or `wtype` (keyboard-only fallback) is on PATH.

First confirm what Tendril detected:

```bash
command -v ydotool
command -v wtype
systemctl --user --no-pager status ydotoold.service 2>/dev/null || pgrep -a ydotoold
```

Then run a keyboard-only sequence against a window target that already has focus:

```bash
nix run .#tendril -- --window <window-id> run --json 'send("hello from Tendril")'
```

Expected success:

- Tendril returns `status: success` with `action_count: 1`.
- The `notes` field names which helper backed the dispatch (`ydotool` or `wtype`) and reminds you that Wayland focus transfer is compositor-mediated.

For pointer events, ensure `ydotool` is installed and `ydotoold` is running, then exercise a click against a display target:

```bash
nix run .#tendril -- --display <display-id> run --json 'lclick(100,100)'
```

Expected failure modes that signal a real configuration issue:

- `ydotoold_unavailable` — install / start the `ydotoold` daemon and make sure the invoking user can reach `$YDOTOOL_SOCKET` (or the default `/tmp/.ydotool_socket`).
- `unsupported_capability` mentioning both `ydotool` and `wtype` — install at least one of the helper tools to enable Wayland input.
- `unsupported_capability` saying that the detected backend is `wtype` and the request contains pointer events — install `ydotool` (and run `ydotoold`) to add pointer support.
### 5. MCP stdio smoke check

Use the same framing helper pattern as the macOS validation page:

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

- the first response includes `"serverInfo":{"name":"tendril"`, and
- the tool list includes `list`, `capture`, and `run`.

## Troubleshooting

### `list` says no supported backend was detected

Match the missing tool to the compositor family you are actually running:

- Hyprland → `hyprctl`
- sway → `swaymsg`
- wlroots display fallback → `wlr-randr`

Tendril's generic Wayland discovery is intentionally explicit about this matrix; it does not guess across unrelated compositor APIs.

### `capture` mentions `xdg-desktop-portal` and `grim`

That means:

1. Tendril tried the preferred portal screenshot path first, and
2. either no screenshot portal backend was available, or the portal path failed and Tendril had to explain the compatibility fallback.

Recommended order of investigation:

1. confirm `xdg-desktop-portal.service` is running in the user session,
2. confirm the compositor-specific portal backend package is installed,
3. rerun the capture command, and only then
4. consider `grim` as a compatibility fallback for the current session.

### `run` says no Wayland input backend is available

That now indicates a missing helper tool rather than an architectural limitation. Install one of:

- `ydotool` (preferred, full keyboard + pointer support; also requires the `ydotoold` daemon to be running and reachable via its socket), or
- `wtype` (keyboard-only fallback for wlroots compositors such as Hyprland and sway).

Then rerun the same command. See the `bd-408572` row of the runtime dependency audit for the supported helper-tool matrix.
