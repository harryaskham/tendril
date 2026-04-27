# bd-0fdcd9 findings

Current-main normal Firefox page-text double-click copy is valid when the gesture targets the actual page-text span center.

- Smoke command: `scripts/tendril-headless.sh --name bd-0fdcd9-page-text --browser firefox --tendril-bin ./target/debug/tendril --artifact-dir summaries/hfgax0acyju3uypm/bd-0fdcd9-page-text page-text-clipboard-smoke`
- Target proof word: `doublewordclipboard`
- Geometry-derived Tendril coordinate: `dblclick(399,371)`
- Page copy state: `copySelection=doublewordclipboard`
- OS clipboard state: `tendril clipboard get --selection clipboard` returned `text=doublewordclipboard` from X11 owner `0x60003a`.

The discarded controller artifact showed why the previous run was misleading: a DOM `copy` event is not enough proof. The focused smoke now records page selection through Marionette and separately reads the X11 `CLIPBOARD`, so a future `copy` event with wrong selected text or `clipboard_selection_unowned` fails instead of passing.
