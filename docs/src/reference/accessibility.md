# Accessibility element contract

Tendril exposes platform-native accessibility metadata through the same stateless
`list-elements` and `run` workflow on every desktop stack:

1. Discover a target with `tendril list --json`.
2. Discover semantic elements with `tendril --window <id> list-elements --json`
   or `tendril --display <id> list-elements --json`.
3. Activate an element with `tendril --window <id> run 'click(<element-id>)'`,
   optionally followed by ordinary DSL input such as `send("text")`.

Element IDs are snapshot-local. They are deterministic for a stable accessibility
tree and are intended to be refreshed whenever the UI changes. The `run` command
resolves `click(<id>)`, `press(<id>)`, and `element(<id>)` by listing elements for
the target again, finding the matching element, taking the center of its bounds,
and dispatching an ordinary target-relative pointer click through the platform
input backend.

## Output shape

Each element uses the shared `ElementDescriptor` JSON shape:

| Field | Meaning |
|---|---|
| `id` | Snapshot-local ID accepted by `click(<id>)`, `press(<id>)`, and `element(<id>)`. |
| `role` | Normalized semantic role, lower-case with spaces/dashes converted to underscores. |
| `name` | Human-readable label from the platform accessibility API. |
| `description` | Optional accessibility description. |
| `value` | Optional current value where the platform exposes one cheaply. |
| `bounds` | Optional source-space screen bounds. Bounds are required for element-click resolution. |
| `target` | The window/display selector that should be used for follow-up commands. |
| `path` | Hierarchical ancestry labels ending in the element name. |
| `actions` | Semantic actions reported by the backend plus `click`/`press` when bounds are clickable. |
| `app_name` / `process_id` | Best-effort owning application metadata. |

## Role taxonomy

Backends should preserve useful native semantics while normalizing spelling into a
portable vocabulary. Current common roles include:

- `window`, `application`, `frame`, `dialog`, `panel`
- `push_button`, `toggle_button`, `radio_button`, `check_box`
- `text`, `entry`, `password_text`, `label`, `paragraph`
- `list`, `list_item`, `table`, `table_cell`, `tree`, `tree_item`
- `menu`, `menu_item`, `menu_bar`, `combo_box`
- `slider`, `scroll_bar`, `progress_bar`, `spin_button`
- `link`, `image`, `icon`, `separator`, `status_bar`

Unknown native roles are still emitted after normalization rather than discarded.
Consumers should treat role strings as hints, not as an exhaustive enum.

## Platform backends

### macOS

The macOS backend uses the Accessibility API for window contents. It queries the
target process, walks `AXChildren`, emits `AXRole`/`AXTitle`/`AXDescription` and
screen-space `AXPosition`/`AXSize`, then relies on the shared input resolver for
`click(<id>)`.

### Linux/X11

The X11 backend first tries the same AT-SPI accessibility bus used by the
Wayland path, walking `org.a11y.atspi.Accessible` trees and filtering elements to
the requested window/display bounds. When AT-SPI is unavailable or an X11
application does not publish accessibility metadata, Tendril falls back to the X
window tree via `xwininfo` as a pragmatic surface-level element source. Both
paths follow the same output shape and click resolver contract.

### Linux/Wayland

The Wayland backend uses AT-SPI on the accessibility bus:

- It reads `AT_SPI_BUS_ADDRESS` when present or asks `org.a11y.Bus.GetAddress`
  on the session bus.
- It calls `org.a11y.atspi.Registry.GetApplications` and walks each
  `org.a11y.atspi.Accessible` tree.
- For window-scoped listing, applications are matched by AT-SPI application ID
  when it matches the compositor-discovered process ID; otherwise elements are
  filtered by target bounds.
- For display-scoped listing, all accessible applications with bounds intersecting
  the display are included.
- Bounds come from `org.a11y.atspi.Component.GetExtents` in screen coordinates.
  The shared `run` resolver converts those into target-relative pointer clicks.

If AT-SPI is unavailable or an application does not publish accessibility
metadata, Tendril falls back to compositor-discovered surface roots and includes a
note in the `list-elements` output.

## DSL interaction contract

The element-aware forms are aliases for a resolved left-click at the element
center:

```bash
tendril --window <id> run 'click(3)'
tendril --window <id> run 'press("submit-button")'
tendril --window <id> run 'element(menu/file/open),send("hello")'
```

After element resolution, the backend receives only ordinary pointer/keyboard
DSL actions. This keeps platform-specific accessibility discovery separate from
input dispatch and lets Wayland reuse the existing `ydotool`/`wtype` input
backends for click and text entry.
