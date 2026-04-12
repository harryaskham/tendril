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
| Hyprland | `hyprctl` | `xdg-desktop-portal` screenshot backend (for example `xdg-desktop-portal-hyprland`) | `grim` | Not supported |
| sway | `swaymsg` | `xdg-desktop-portal` screenshot backend (commonly `xdg-desktop-portal-wlr`) | `grim` | Not supported |
| wlroots-compatible compositor with output enumeration only | `wlr-randr` | `xdg-desktop-portal` screenshot backend (commonly `xdg-desktop-portal-wlr`) | `grim` | Not supported |

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

### 4. Verify that generic input remains explicitly unsupported

Wayland input injection is still compositor-specific in this repository.

Run:

```bash
nix run .#tendril -- --window <window-id> run --json 'send("hello from Tendril")'
```

Expected result:

- Tendril returns a structured `unsupported_capability` error.
- The error guidance should point you at X11 or a future compositor-specific backend rather than pretending generic Wayland input is available.

### 5. MCP stdio smoke check

Use the same framing helper pattern as the macOS validation page:

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

### `run` still fails on Wayland

That is expected for the generic Linux adapter today. This bead only improves discovery/capture packaging and diagnostics; it does not claim generic Wayland input injection support.
