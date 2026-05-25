# Android device backend

Tendril can drive a connected Android device or emulator through ADB:

```bash
tendril --android sgu24:5555 list --json
tendril --android emulator-5554 list-elements --json
tendril --android auto capture -o screenshot.png
tendril --android sgu24:5555 run 'click(231,1905),wait(500ms),press("Monitor")'
tendril --android auto list --all-apps --json
tendril --android emulator-5554 run 'launch("com.example"),assert_visible("Ready")'
tendril --android sgu24:5555 run 'notifications(),wait(500ms),back()'
```

`--android auto` selects the single connected `adb devices` entry and fails clearly when zero or multiple devices are available. If the flag is omitted, `TENDRIL_ANDROID_SERIAL` provides the same serial selection. The serial can be a local emulator id such as `emulator-5554` or an ADB-over-TCP endpoint such as `192.0.2.10:5555`; `TENDRIL_ADB_BIN` can point Tendril at a non-default adb binary.

## Supported MVP commands

- `list` checks `adb get-state`, reads device metadata such as `wm size`, `wm density`, model, focused window/activity, active app, recent/switchable apps, and optionally all launchable apps with `--all-apps`. It returns an Android display target named `android:<serial>` plus app/window-style targets named `android:<serial>:app:<package>` for known apps.
- `list-elements` runs `adb shell uiautomator dump /sdcard/tendril-window.xml`, reads the XML back with `adb exec-out cat`, and maps UIAutomator nodes into Tendril element descriptors with text/content-desc/resource-id/class/package/bounds and clickability flags.
- `capture` runs `adb exec-out screencap -p` and returns the PNG in Tendril's normal capture envelope. `-o/--output` writes the decoded PNG.
- Every Android invocation writes debug artifacts under `TENDRIL_ANDROID_ARTIFACT_DIR` when set, or a per-run directory under the system temp directory. The MVP writes `commands.log`, `ui.xml` for element dumps, `screenshot.png` for captures, and `window.txt` when focus metadata is available.
- `run` maps Tendril's existing DSL onto `adb shell input` primitives:
  - `click(x,y)` / `lclick(x,y)` → `input tap`
  - `dblclick(x,y)` → two taps
  - `drag(x0,y0,x1,y1)` and `scroll(x,y,dy)` → `input swipe`
  - `send("text")` or plain text → `input text`
  - `Return`, `Back`, `Home`, `Recents`, `Assistant`, `Wakeup`, `Menu`, `Tab`, `Space`, `Escape`, volume/power keys, and numeric keyevent codes → `input keyevent`
  - `click(<element-id>)` can target IDs returned by `list-elements`, or exact text/content-desc/resource-id values from the current UI dump.
  - `tap_text("Monitor")`, `tap_desc("Route monitor")`, and `tap_resource("app:id/monitor")` are selector aliases over UIAutomator text/content-desc/resource-id.
  - `scroll_until("Done")` repeatedly swipes and re-dumps UIAutomator until the selector is visible, then taps it.
  - `assert_visible("Ready")` and `assert_absent("Error")` validate UI state without tapping.
  - `launch("<package>")`, `open("<package>")`, `switch("<package>")`, `press("launch:<package>")`, or `press("package:<package>")` launches/switches an app through `monkey -p <package> -c android.intent.category.LAUNCHER 1`.
  - `back()`, `home()`, `recents()`, and `assistant()` are Android system navigation actions.
  - `notifications()` and `quicksettings()` expand the notification shade / quick settings via `cmd statusbar`; `status()` writes a `status.json` artifact with the current device/app summary.

## Safety model

The backend is intentionally limited to ordinary UI-driving primitives. It does not factory reset, uninstall, clear app data, force-stop unrelated packages, or kill processes. Explicit app launch/switch and system navigation/shade actions are allowed because agents need them for ordinary Android UI automation.

## Debugging notes

The backend surfaces ADB failures as structured Tendril errors. Common causes are:

- `adb` is missing from `PATH`.
- The serial is offline or unauthorized.
- `--android auto` found zero or multiple devices.
- UIAutomator cannot dump the current surface, for example on some lockscreen/system overlay states.

For robust workflows, start with `list --json`, then `list-elements --json`, then use element IDs or coordinate taps from the observed bounds.
