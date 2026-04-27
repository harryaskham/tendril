# bd-646769 reproduction notes

- Reproduced the reported workflow on current main after bd-c0d068 had landed using Firefox in the headless X11 environment.
- The original gesture `drag(95,350,850,350),wait(500ms),hold(ctrl),c,release(ctrl),wait(700ms)` focused the textarea and fired the page `copy` handler, but Marionette state showed `selectionStart == selectionEnd == 30` and `selected == ""`.
- Because Firefox had no non-empty textarea selection to publish, it did not take ownership of the X11 `CLIPBOARD` selection; `tendril clipboard get --selection clipboard` correctly returned `clipboard_selection_unowned`.
- The smoke in this directory uses a verified text-baseline gesture, `drag(95,328,850,328),wait(500ms),hold(ctrl),c,release(ctrl),wait(700ms)`, which leaves the full proof text selected, fires the page copy event, and returns `select-drag-clipboard-proof-ok` through `tendril clipboard get`.
- `clipboard_selection_unowned` now includes an actionable diagnostic for this Firefox case: a page copy event can fire for an empty textarea selection, so callers should verify visible selection / `selectionStart != selectionEnd`, use a text-baseline drag, drag backward, or use Ctrl+A before Ctrl+C.
