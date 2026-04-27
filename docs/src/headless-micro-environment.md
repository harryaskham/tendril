# Headless Tendril micro-environment

Tendril ships a Linux/X11 helper for agent browser and OS-control work that must
not touch the operator's real desktop. It starts a disposable 1920x1080 Xvfb
session with a lightweight window manager, browser, and optional shell window,
then points ordinary `tendril list`, `tendril capture`, and `tendril run`
commands at that isolated display.

Use this micro-environment when an agent needs to click, type, navigate a
browser, or capture UI state without stealing focus from the human operator.
Use the real desktop only when the task explicitly needs the operator's current
session, native window manager, hardware acceleration, or platform permission
prompts.

## What it provides

- Default deterministic desktop: `1920x1080x24`, scale factor 1:1.
- Isolated `DISPLAY` chosen from `:90`-`:199` unless `--display` is supplied.
- `Xvfb` with XTEST enabled, so Tendril's X11 input backend can type/click.
- Lightweight window manager when available (`openbox`, `fluxbox`,
  `matchbox-window-manager`, or `twm`).
- Browser when available (`chromium`, `chromium-browser`, `google-chrome`, or
  `firefox`) plus basic shell utilities. Chromium-based browsers are forced onto
  the X11 Ozone backend so a host Wayland session cannot steal the browser away
  from the Xvfb display.
- Resource guardrails for the default Chromium path: fixed-size Xvfb screen,
  disabled background/sync/extension work, disabled crashpad/breakpad reporting,
  disabled Chromium sandboxing inside the disposable Xvfb sandbox, and a renderer
  process cap of 2 (`--browser-renderers <n>` to tune, `0` to omit). Set
  `TENDRIL_HEADLESS_CHROMIUM_SANDBOX=true` if you need to validate with the
  browser sandbox enabled.
- Clean lifecycle commands: `start`, `env`, `inspect`, `reset`, `stop`,
  `smoke`, `firefox-upload`, and `file-upload-smoke`.
- Smoke artifacts written to a git-trackable summaries directory by default:
  `summaries/$CACOPHONY_AGENT/` or `summaries/manual/`.

The helper is intentionally process-scoped and disposable. It is not a Tendril
daemon and it does not add hidden Tendril state. The only state file is the
helper's lifecycle metadata under `$XDG_RUNTIME_DIR/tendril-headless/<name>/` so
subsequent `env`, `inspect`, `reset`, and `stop` commands can find the sandbox.

## Copy-paste smoke test

From a checkout, build the latest Tendril binary and run the smoke workflow:

```bash
cargo build -p tendril
scripts/tendril-headless.sh --name smoke --tendril-bin ./target/debug/tendril smoke
```

If you want to force execution through the latest Nix package output instead of
a system Tendril binary, run the packaged helper directly:

```bash
nix run .#tendril-headless -- --name smoke smoke
```

You can also keep using the checkout script while routing Tendril CLI calls
through Nix explicitly:

```bash
scripts/tendril-headless.sh \
  --name smoke \
  --tendril-bin 'nix run .#tendril --' \
  smoke
```

What the smoke does:

1. Starts or reuses an isolated Xvfb desktop.
2. Opens a local `Tendril Smoke Browser` page in the browser.
3. Waits until `tendril --json list` sees both the 1920x1080 display and a
   browser window. A shell, window manager helper, or any other non-browser
   window is not accepted as proof of browser control.
4. Captures the display with explicit `--max-width 1920 --max-height 1080`.
5. Runs browser-visible input against the discovered browser window:
   `send("tendril browser control confirmed"),Return,wait(500ms)`.
6. When the helper launched XTerm, runs `hold(shift),Insert,release(shift),wait(100ms)`
   against that terminal and records the run JSON. This catches X11 backend
   regressions where the Insert key cannot be dispatched for terminal paste
   shortcuts.
7. Captures the controlled browser window after input so the artifact visibly
   shows the typed value and confirmation banner.
8. Writes artifacts under `summaries/<agent-id>/` and runs `git add` for that
   directory unless `--no-git-add-artifacts` is set.
9. Stops the sandbox when the smoke command started it.

Expected artifacts:

```text
summaries/<agent-id>/smoke-list.json
summaries/<agent-id>/smoke-capture.json
summaries/<agent-id>/smoke-run.json
summaries/<agent-id>/smoke-xterm-shift-insert-run.json (when XTerm is available)
summaries/<agent-id>/smoke-display.png
summaries/<agent-id>/smoke-browser-after.png
summaries/<agent-id>/smoke-browser-after-capture.json
summaries/<agent-id>/smoke-manifest.txt
```

The `.png` files are deliberately in `summaries/` rather than `/tmp`;
Cacophony summary collectors and `/tmp/watch-captures.sh` can then surface the
captures to the operator. The display capture proves the isolated desktop is
reachable; the browser-after capture is the proof of in-browser control.

## Browser↔OS clipboard smoke test

For deterministic clipboard transfer in the headless X11 desktop, prefer the
explicit Tendril clipboard helper over terminal-only paste fallbacks. X11
clipboard data is owned by a live client process; middle-click primary paste and
Shift+Insert can exercise a different selection or toolkit binding than the
browser's Ctrl+C clipboard. The helper makes that state visible and returns a
structured error if no owner responds.

Focused end-to-end smoke:

```bash
cargo build -p tendril
scripts/tendril-headless.sh \
  --name clipboard-smoke \
  --browser firefox \
  --tendril-bin ./target/debug/tendril \
  --artifact-dir "summaries/${CACOPHONY_AGENT:-manual}/clipboard-smoke" \
  clipboard-smoke
```

What the smoke proves:

1. Starts the isolated Xvfb desktop with Firefox and XTerm available.
2. Opens a local page with proof text `browser-to-os-clipboard-control-ok`.
3. Uses Tendril input to click the Firefox textarea and dispatch Ctrl+A/C.
4. Reads the OS-side X11 `CLIPBOARD` selection with
   `tendril clipboard get --json` and asserts it equals the browser proof text.
5. Serves OS text `os-to-browser-clipboard-control-ok` with
   `tendril clipboard set --serve-ms ...` while Tendril dispatches Ctrl+V into
   Firefox.
6. Copies the Firefox paste target back out and reads it with
   `tendril clipboard get --json`, proving the OS→browser transfer as well.
7. Writes JSON, PNG, stderr, and manifest artifacts under the requested
   `summaries/` directory.

Manual recipe after starting the environment:

```bash
# Browser -> OS: perform the browser copy through Tendril input, then inspect
# the X11 CLIPBOARD owner through Tendril instead of assuming terminal paste.
tendril --json --window "$firefox_window" run \
  'lclick(<textarea_x>,<textarea_y>),hold(ctrl),a,release(ctrl),hold(ctrl),c,release(ctrl),wait(500ms)'
tendril --json clipboard get --selection clipboard --timeout-ms 3000

# OS -> Browser: keep the helper alive while the browser requests the paste.
(tendril --json clipboard set --text 'hello from OS' --serve-ms 8000 >clipboard-set.json) &
server_pid=$!
tendril --json --window "$firefox_window" run \
  'lclick(<target_x>,<target_y>),hold(ctrl),v,release(ctrl),wait(500ms)'
wait "$server_pid"
```

If `clipboard get` returns `clipboard_selection_unowned`, the source application
did not own the requested X11 selection. If it returns
`clipboard_conversion_failed` or `clipboard_incr_not_supported`, the owner did
not provide a direct plain-text selection; retry with a smaller text selection
or file a platform/toolkit-specific clipboard backend bead with the JSON error
details.

## Firefox contextmenu smoke test

Use the focused contextmenu smoke when validating Linux/X11 right-click delivery
into Firefox content. It prevents the browser menu, records the page-observed DOM
`contextmenu` event through Marionette, and fails if Tendril only reports a
successful `run` envelope without the page changing state.

```bash
cargo build -p tendril
scripts/tendril-headless.sh \
  --name contextmenu-smoke \
  --browser firefox \
  --tendril-bin ./target/debug/tendril \
  --artifact-dir "summaries/${CACOPHONY_AGENT:-manual}/contextmenu-smoke" \
  contextmenu-smoke
```

The smoke asserts that `rclick(220,390),wait(900ms)` returns success with
Firefox focus transfer and that the page state reports
`contextMenuObserved=true`, `contextMenuButton=2`, and title
`Tendril Context Menu Hit`.

## Firefox nested scroll smoke test

Use the focused scroll smoke when validating Linux/X11 wheel delivery into a
pointer-local nested Firefox scroll region. It records page state through
Marionette and fails if Tendril reports a successful `run` envelope without the
nested scroll pane moving.

```bash
cargo build -p tendril
scripts/tendril-headless.sh \
  --name scroll-smoke \
  --browser firefox \
  --tendril-bin ./target/debug/tendril \
  --artifact-dir "summaries/${CACOPHONY_AGENT:-manual}/scroll-smoke" \
  scroll-smoke
```

The smoke asserts that `scroll(220,420,8),wait(900ms)` returns success with
Firefox focus transfer and that the page state reports `scrollObserved=true`,
`scrollTop > 0`, and title `Tendril Scroll Hit`.

## Manual lifecycle

Start an environment and export its display into the current shell:

```bash
eval "$(scripts/tendril-headless.sh --name browser-task start)"
```

Inspect targets without touching the real desktop:

```bash
tendril --json list
```

Capture the isolated display into the summaries directory:

```bash
mkdir -p "summaries/${CACOPHONY_AGENT:-manual}"
display_id=$(tendril --json list | python3 -c '
import json, sys
for target in json.load(sys.stdin)["data"]["targets"]:
    if target["kind"] == "display":
        print(target["id"])
        break
')
tendril --json --display "$display_id" capture \
  --max-width 1920 --max-height 1080 \
  -o "summaries/${CACOPHONY_AGENT:-manual}/browser-task-display.png"
git add "summaries/${CACOPHONY_AGENT:-manual}/browser-task-display.png"
```

Run input against a discovered browser window:

```bash
window_id=$(tendril --json list | python3 -c '
import json, sys
targets=json.load(sys.stdin)["data"]["targets"]
for target in targets:
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if target["kind"] == "window" and any(token in haystack for token in ("chrom", "firefox")):
        print(target["id"])
        break
')
# Capture first and choose the visible address bar coordinates in source-space.
# In the default 1920x1080 headless browser, the bar is near the top chrome;
# adjust from your capture instead of relying on a fixed page coordinate.
tendril --json --window "$window_id" capture -o "summaries/${CACOPHONY_AGENT:-manual}/browser-before-nav.png"
tendril --json --window "$window_id" run \
  'lclick(<address_bar_x>,<address_bar_y>),hold(ctrl),a,release(ctrl),send("https://example.com"),Return,wait(1000ms)'
tendril --json --window "$window_id" capture -o "summaries/${CACOPHONY_AGENT:-manual}/browser-after-nav.png"
```

Do not use `hold(ctrl),l,release(ctrl),send("URL"),Return` as the browser-navigation preflight on Linux/X11. Firefox can leave focus inside an existing page input even though Tendril successfully dispatched the chord, causing the URL to be typed into the page. Click the visible address bar and recapture/verify the page changed before continuing.

## Firefox file uploads

In the packaged Xvfb desktop, Firefox's native file picker is not a reliable
Tendril target. A Tendril click on a visible `<input type="file">` Browse button
can return a successful `run` result, but `tendril list` still exposes only the
Firefox top-level window (and the optional XTerm shell), with no separate Open
File/File Chooser window to capture or control. Treat that as a browser-native
modal boundary rather than an agent-controllable OS dialog.

Use the explicit Firefox helper when an agent must upload a local file in the
headless browser. The helper starts Firefox with Marionette enabled, then uses
WebDriver's file-input path injection for the selected element. It does not use
terminal side effects; follow it with Tendril capture artifacts of the browser
window so the page itself proves the upload succeeded.

Focused end-to-end smoke:

```bash
cargo build -p tendril
scripts/tendril-headless.sh \
  --name file-upload-smoke \
  --browser firefox \
  --tendril-bin ./target/debug/tendril \
  --artifact-dir "summaries/${CACOPHONY_AGENT:-manual}/file-upload-smoke" \
  file-upload-smoke
```

The smoke:

1. starts a Firefox-backed headless desktop with a recorded Marionette port,
2. opens a checkout-local upload page,
3. captures the upload form,
4. clicks the native Browse button with Tendril and records that no separate
   chooser target appears in `list`,
5. uploads `upload-source/upload-proof.txt` through `firefox-upload`, and
6. captures the browser after upload, where the page displays `Uploaded file
   confirmed` and the proof file contents.

For a custom page in a running Firefox environment:

```bash
eval "$(scripts/tendril-headless.sh --name browser-upload --browser firefox start)"

scripts/tendril-headless.sh \
  --name browser-upload \
  --navigate-url 'file:///absolute/path/to/form.html' \
  --file-input-selector 'input[type="file"]' \
  --upload-file '/absolute/path/to/proof.txt' \
  --helper-output "summaries/${CACOPHONY_AGENT:-manual}/firefox-upload-helper.json" \
  firefox-upload

window_id=$(tendril --json list | python3 -c '
import json, sys
for target in json.load(sys.stdin)["data"]["targets"]:
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if target["kind"] == "window" and "firefox" in haystack:
        print(target["id"])
        break
')
tendril --json --window "$window_id" capture \
  -o "summaries/${CACOPHONY_AGENT:-manual}/firefox-upload-after.png"
```

Reset or stop the environment:

```bash
scripts/tendril-headless.sh --name browser-task reset
scripts/tendril-headless.sh --name browser-task stop
```

## Resource and safety notes

- Use a unique `--name` per agent/task so multiple workers do not share runtime
  directories or display numbers accidentally.
- Keep the Chromium renderer cap low for routine agent tasks; raise it only for
  pages that genuinely need more renderer processes.
- `start` reuses a live environment with the same name; `reset` destroys and
  recreates it.
- Runtime logs stay under `$XDG_RUNTIME_DIR/tendril-headless/<name>/logs/` and
  are removed by `stop` unless `--keep-runtime` is supplied.
- Smoke capture artifacts are refused if `--artifact-dir` points at `/tmp`,
  `/var/tmp`, or `/run`; use a checkout-local summaries path for captures.
- If a browser exits before Tendril can discover it, the helper logs the early
  exit, preserves diagnostic excerpts for common Chromium crashpad/sandbox
  failures, tries the next auto-detected browser candidate when `--browser` was
  not explicit, and copies runtime logs to `<artifact-dir>/runtime-logs` on
  smoke failure before cleaning up the sandbox.
- The Nix package installs both `tendril` and `tendril-headless`; release
  archives include both binaries.
- The helper is Linux/X11-first. On macOS and Windows, use the native Tendril
  validation guides or a future VM/microVM sidecar when stronger isolation is
  required.
