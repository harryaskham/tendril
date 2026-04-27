# bd-765fa3 X11 Firefox/XTerm occluder smoke

Focused regression smoke for X11 browser-window capture with an overlapping XTerm occluder.

Setup:
- Isolated Xvfb/Openbox desktop via `scripts/tendril-headless.sh --name bd-765fa3 --browser firefox`.
- Firefox target: `0x600016`.
- XTerm target: `0x40000c`.
- Display target: `1`.
- `xterm-raise-run.json` uses `--no-restore-focus` so XTerm remains stacked above Firefox.

Artifacts:
- `list-before.json`: discovered Firefox, XTerm, and display targets.
- `display-with-xterm-occluding-firefox.png`: display-scoped capture proving XTerm visibly overlaps Firefox on the root window.
- `firefox-window-after-fix.png`: Firefox window-scoped capture produced by the fixed X11 path; it is unoccluded despite the display capture proving the occluder was above it.
- `firefox-window-after-fix.json`: successful capture envelope for the fixed Firefox target capture.
- `*.stats.json`: focused PNG near-black statistics from a small stdlib PNG decoder.

Expected fixed behavior:
Tendril detects mapped X11 windows stacked above the requested target. When any overlap the target, it treats a normal target-drawable capture as unsafe, raises Firefox, captures the target bounds from the root window, validates that the fallback is not solid black, and restores the previous X11 focus/pointer state. If that fallback cannot produce usable pixels, the error message includes the occlusion/fallback context instead of returning a normal-looking success.
