#!/usr/bin/env python3
"""Fill the marked regions of the hand-written pages in site/.

Pages are ordinary HTML edited in place. This rewrites, deterministically:

  <!-- shell:head --> … <!-- /shell:head -->   metadata, fonts, CSS and JS
  <!-- shell:top -->  … <!-- /shell:top -->    skip link, brand and the film-edge navigation
  <!-- shell:rail --> … <!-- /shell:rail -->   this page's sections, numbered like film frames
  <!-- shell:foot --> … <!-- /shell:foot -->   colophon
  <!-- fig:NAME -->   … <!-- /fig -->          inline SVG from scripts/site/figures.py
  <pre><code>…</code></pre>                     one block per line, so wrapped lines hang-indent
  <code>short</code>                           short inline code gets class="nb" (never wraps)

It also writes redirect pages for the URLs of the previous (mdBook) site.

  python3 scripts/site/build.py           # rewrite pages
  python3 scripts/site/build.py --check   # fail if anything is stale
"""

from __future__ import annotations

import html
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import made  # noqa: E402  (the a.skh.am made-by meta and seal)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SITE = ROOT / "site"
sys.path.insert(0, str(HERE))

NAME = "Tendril"
BASE = "/tendril/"
FONTS = ["schibsted-grotesk", "fragment-mono"]
THEME = "#0c0c0d"
FOOT = "Tendril 0.0.4 · Rust · stateless desktop control for agents"

# file, nav label, <title>, meta description
PAGES = [
    ("index.html", "Overview", "Tendril · eyes and hands on a desktop, for agents",
     "Tendril is a stateless Rust CLI that lets agents see and drive desktops: list windows, displays and cameras, "
     "capture screenshots, read accessibility elements, type, click and scroll, on Linux, macOS, Windows and Android."),
    ("guide.html", "Guide", "Guide · discover, capture, act, verify",
     "The Tendril loop in practice: list targets, capture with coordinate scaling, list semantic elements, drive "
     "input with the run language, restore focus, record audio and make shell aliases."),
    ("platforms.html", "Platforms", "Platforms · Linux, macOS, Windows, Android and remote",
     "What Tendril supports on each platform: X11 and Wayland backends, macOS permissions, native Windows, Android "
     "over ADB, SSH remotes, the WSL tunnel, a disposable headless desktop and runtime dependencies."),
    ("agents.html", "Agents", "Agents · MCP tools, envelopes and the execution lock",
     "Tendril for agents: the MCP tool surface, JSON envelopes and structured errors, the host-wide execution lock "
     "and queue, safety guards, and the capture-act-verify discipline."),
    ("reference.html", "Reference", "Reference · commands, the run language and configuration",
     "Every Tendril command and flag, the run language's actions, configuration, environment variables, updates and "
     "the Rust API."),
]
EXTRA = [("404.html", "", "Not found · Tendril", "This page does not exist.")]

# Old mdBook URL -> new page (and section).
REDIRECTS = {
    "quickstart.html": "guide.html",
    "mcp.html": "agents.html#mcp",
    "headless-micro-environment.html": "platforms.html#headless",
    "linux-x11-operator-validation.html": "platforms.html#linux",
    "linux-wayland-operator-validation.html": "platforms.html#linux",
    "macos-operator-validation.html": "platforms.html#macos",
    "cli/index.html": "reference.html#commands",
    "cli/list.html": "guide.html#list",
    "cli/capture.html": "guide.html#capture",
    "cli/run.html": "guide.html#run",
    "cli/listen.html": "guide.html#listen",
    "cli/alias.html": "guide.html#alias",
    "cli/remote.html": "platforms.html#remote",
    "cli/update.html": "reference.html#updates",
    "cli/version.html": "reference.html#commands",
    "reference/index.html": "reference.html",
    "reference/accessibility.html": "guide.html#elements",
    "reference/android.html": "platforms.html#android",
    "reference/api.html": "reference.html#api",
    "reference/configuration.html": "reference.html#config",
    "reference/execution-lock.html": "agents.html#lock",
    "reference/json-envelopes.html": "agents.html#envelopes",
    "reference/platform-support.html": "platforms.html",
    "reference/publishing.html": "reference.html#development",
    "reference/runtime-dependencies.html": "platforms.html#dependencies",
    "reference/nix-runtime-dependency-audit.html": "platforms.html#dependencies",
    "reference/pi-cacophony-mcp-contract.html": "agents.html#mcp",
}


def head(title: str, desc: str, base: str | None = None) -> str:
    out = [f'<base href="{base}">'] if base else []
    out += [
        '<meta charset="utf-8">', made.META,
        '<meta name="viewport" content="width=device-width, initial-scale=1">',
        f"<title>{html.escape(title)}</title>",
        f'<meta name="description" content="{html.escape(desc, quote=True)}">',
        f'<meta name="theme-color" content="{THEME}">',
        '<meta name="color-scheme" content="light">',
        f'<meta property="og:title" content="{html.escape(title, quote=True)}">',
        f'<meta property="og:description" content="{html.escape(desc, quote=True)}">',
        '<meta property="og:type" content="website">',
        '<link rel="icon" href="assets/favicon.svg" type="image/svg+xml">',
    ]
    out += [f'<link rel="preload" href="assets/fonts/{f}.woff2" as="font" type="font/woff2" crossorigin>' for f in FONTS]
    out += ['<link rel="stylesheet" href="assets/site.css">', '<script src="assets/site.js" defer></script>']
    return "\n".join(out)


def top(current: str) -> str:
    cur = ' aria-current="page"'
    links = "".join(f'<a href="{f}"{cur if f == current else ""}>{label}</a>' for f, label, _, _ in PAGES)
    return "\n".join([
        '<a class="skip" href="#main">Skip to content</a>',
        '<header class="edge"><div class="edge-in">',
        '  <a class="brand" href="index.html" aria-label="Tendril home"><b>tendril</b><small>0.0.4</small></a>',
        f'  <nav class="nav" aria-label="Pages">{links}<a href="api/tendril/index.html">API</a></nav>',
        "</div></header>",
    ])


H2 = re.compile(r'<h2 id="([a-z0-9-]+)"[^>]*>(.*?)</h2>', re.S)


def rail(src: str) -> str:
    items = [f'<li><a href="#{m.group(1)}">{html.escape(html.unescape(re.sub(r"<[^>]+>", "", m.group(2))).strip())}</a></li>'
             for m in H2.finditer(src)]
    if not items:
        return ""
    return '<nav aria-label="On this page"><p>Frames</p>\n<ol>\n' + "\n".join(items) + "\n</ol></nav>"


def foot() -> str:
    return "\n".join([
        '<footer class="foot">',
        f"  <span>{FOOT}</span>",
        '  <span>Type: Schibsted Grotesk and Fragment Mono, <a href="assets/fonts/LICENSE.txt">OFL 1.1</a></span>',
        f'<div class="made-row" style="flex:1 0 100%;width:100%;margin-top:14px;line-height:0">{made.SEAL}</div>', "</footer>",
    ])


def lines(m: re.Match) -> str:
    open_, body = m.group(1), m.group(2)
    if body.startswith('<span class="ln">'):
        body = body[len('<span class="ln">'):-len("</span>")].replace('</span><span class="ln">', "\n")
    body = body.replace("<wbr>", "")
    rows = body.split("\n")
    for row in rows:
        if len(re.findall(r"<span\b", row)) != row.count("</span>"):
            return m.group(0)
    rows = [re.sub(r",(?![^<]*>)", ",<wbr>", row) for row in rows]
    return open_ + "".join(f'<span class="ln">{row}</span>' for row in rows) + "</code></pre>"


def nowrap_short_code(src: str) -> str:
    parts = re.split(r"(<pre\b.*?</pre>|<svg\b.*?</svg>)", src, flags=re.S)
    for i in range(0, len(parts), 2):
        parts[i] = re.sub(r"<code>([^<]{1,28})</code>", r'<code class="nb">\1</code>', parts[i])
    return "".join(parts)


def region(src: str, name: str, body: str) -> str:
    pat = re.compile(rf"(<!-- shell:{name} -->)(.*?)(<!-- /shell:{name} -->)", re.S)
    if not pat.search(src):
        return src
    return pat.sub(lambda m: f"{m.group(1)}\n{body}\n{m.group(3)}", src, count=1)


def render(file: str, title: str, desc: str, src: str, figs: dict, used: set) -> str:
    src = region(src, "head", head(title, desc, BASE if file == "404.html" else None))
    src = region(src, "top", top(file))
    src = region(src, "rail", rail(src))
    src = region(src, "foot", foot())
    src = re.sub(r"(<pre[^>]*><code>)(.*?)</code></pre>", lines, src, flags=re.S)
    src = nowrap_short_code(src)

    def fig(m: re.Match) -> str:
        name = m.group(1)
        if name not in figs:
            raise SystemExit(f"{file}: unknown figure {name!r}")
        used.add(name)
        return f"<!-- fig:{name} -->\n{figs[name]()}\n<!-- /fig -->"

    return re.sub(r"<!-- fig:([a-z0-9-]+) -->.*?<!-- /fig -->", fig, src, flags=re.S)


def redirect(old: str, new: str) -> str:
    target = "../" * old.count("/") + new
    return "\n".join([
        "<!doctype html>", '<html lang="en">', "<head>", '<meta charset="utf-8">', made.META, f"<title>Moved · {NAME}</title>",
        '<meta name="robots" content="noindex">', f'<meta http-equiv="refresh" content="0; url={target}">',
        f'<link rel="canonical" href="{target}">', "</head>", '<body data-redirect="">',
        f'<p>This page moved to <a href="{target}">{new}</a>.</p>', "</body>", "</html>", "",
    ])


def main() -> int:
    check = "--check" in sys.argv[1:]
    import figures  # noqa: E402

    figs = figures.FIGURES
    used: set[str] = set()
    stale = []
    for file, _, title, desc in PAGES + EXTRA:
        path = SITE / file
        if not path.exists():
            continue
        src = path.read_text()
        out = render(file, title, desc, src, figs, used)
        if out != src:
            stale.append(file)
            if not check:
                path.write_text(out)
    for old, new in REDIRECTS.items():
        path = SITE / old
        want = redirect(old, new)
        if not path.exists() or path.read_text() != want:
            stale.append(old)
            if not check:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(want)
    unused = sorted(set(figs) - used)
    if check:
        if stale:
            print("site build: stale (run scripts/site/build.py): " + ", ".join(stale))
            return 1
        if unused:
            print("site build: figures not used by any page: " + ", ".join(unused))
            return 1
        print(f"site build: {len(PAGES) + len(EXTRA)} pages and {len(REDIRECTS)} redirects up to date")
        return 0
    print("site build: wrote " + (", ".join(stale) if stale else "nothing (up to date)"))
    if unused:
        print("site build: unused figures: " + ", ".join(unused))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
