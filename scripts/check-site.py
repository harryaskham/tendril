#!/usr/bin/env python3
"""Validate the static site in site/ before publishing (standard library only).

Checks: pages are built (scripts/site/build.py --check); every page has one <h1>, <main id="main">,
a description, unique ids and one current page key; redirect pages point at real pages; local links,
images, scripts and CSS url()s resolve, including #fragments across pages (links into api/, the
rustdoc added at publish time, are skipped); no root-relative or third-party assets; no symlinks;
byte budgets; and no private details. Addresses, tailnet names, logins and keys are matched by
pattern. Private host and device names are matched by SHA-256 so this public file does not list them.

  python3 scripts/check-site.py
"""

from __future__ import annotations

import hashlib
import re
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"
BASE = "/tendril/"
SKIP_PREFIXES = ("api/",)
PAGE_BUDGET = 160 * 1024
ASSET_BUDGET = 200 * 1024
TOTAL_BUDGET = 3 * 1024 * 1024

FORBIDDEN = [
    r"\b100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d+\.\d+\b",   # tailnet (CGNAT) addresses
    r"\b192\.168\.\d+\.\d+\b", r"\b10\.\d+\.\d+\.\d+\b",
    r"\.ts\.net\b", r"/home/harry\b", r"/Users/harry\b", r"harryaskham@", r"\bharry@", r"@gmail\.com",
    r"BEGIN [A-Z ]*PRIVATE KEY", r"\bAGE-SECRET-KEY-", r"\bgh[pousr]_[A-Za-z0-9]{20}",
]
# sha256(word)[:16] of private host, device and network names.
PRIVATE_WORDS = set("""
a55491c0da39f522
693b286515bd1dd0
9b89025ce7a6d932
e712a02b83a65ca2
69d0bd8119bb0575
fdd823e31be438f6
882010ab667265dc
64e0ff1229ff0070
ee82147236c9bd49
2fcb780b25e013b3
acf9e97160368047
186641861d6d9f2f
db962c64c533179f
c59d7600c2936b1a
d76663720523fd3b
5355589b6790e578
33172798e58e9da9
e99494ca23854244
21aa481893bd9b22
3d955ce8e84b36ba
9893640fede5b54b
1e5f8211207b5eb2
85b433a83b02a983
01d712e4dd828fbc
766443c18fa0c2be
1e49edbef3c0855d
7f1d49243ee66277
9ac64da40357c2bb
d27401c995de2d81
ae0f906a3cc688f0
93bfe4b7de384c75
3f780370dcefe1e7
c478dbd1ded89170
277b42c81ee7e98b
a8ecd2cff40537c9
b3e14c650d20d285
73064a5498513187
""".split())


class Page(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.ids: list[str] = []
        self.refs: list[tuple[str, str]] = []
        self.h1 = 0
        self.main = False
        self.current = 0
        self.desc = False
        self.redirect = False
        self.base: str | None = None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            self.ids.append(a["id"])
        if tag == "h1":
            self.h1 += 1
        if tag == "main" and a.get("id") == "main":
            self.main = True
        if tag == "a" and a.get("aria-current") == "page":
            self.current += 1
        if tag == "meta" and a.get("name") == "description" and a.get("content"):
            self.desc = True
        if tag == "body" and "data-redirect" in a:
            self.redirect = True
        if tag == "base":
            self.base = a.get("href")
            return
        for attr in ("href", "src"):
            if a.get(attr):
                self.refs.append((tag, a[attr]))


def main() -> int:
    errors: list[str] = []
    err = errors.append
    if not (SITE / "index.html").is_file():
        print("check-site: site/index.html is missing")
        return 1
    build = subprocess.run([sys.executable, str(ROOT / "scripts/site/build.py"), "--check"], capture_output=True, text=True)
    if build.returncode:
        err(build.stdout.strip() or build.stderr.strip())

    files = sorted(p for p in SITE.rglob("*") if not p.is_dir())
    total = 0
    for f in files:
        rel = f.relative_to(SITE)
        if f.is_symlink():
            err(f"{rel}: symlink")
            continue
        size = f.stat().st_size
        total += size
        budget = PAGE_BUDGET if f.suffix == ".html" else ASSET_BUDGET
        if size > budget:
            err(f"{rel}: {size} bytes > {budget}")
        if f.suffix in (".html", ".css", ".js", ".svg", ".txt", ".json"):
            text = f.read_text()
            for pat in FORBIDDEN:
                hit = re.search(pat, text)
                if hit:
                    err(f"{rel}:{text.count(chr(10), 0, hit.start()) + 1}: forbidden {hit.group(0)!r}")
            for word in set(re.findall(r"[a-z0-9][a-z0-9-]*", text.lower())):
                if hashlib.sha256(word.encode()).hexdigest()[:16] in PRIVATE_WORDS:
                    err(f"{rel}: a private host or device name ({word[:2]}…)")
    if total > TOTAL_BUDGET:
        err(f"site: {total} bytes > {TOTAL_BUDGET}")

    pages = {f.relative_to(SITE).as_posix(): f for f in files if f.suffix == ".html"}
    parsed: dict[str, Page] = {}
    for name, f in pages.items():
        p = Page()
        p.feed(f.read_text())
        parsed[name] = p
        if p.redirect:
            if len(p.refs) != 2:
                err(f"{name}: a redirect page links exactly one target")
            continue
        if p.h1 != 1:
            err(f"{name}: {p.h1} <h1> elements")
        if not p.main:
            err(f'{name}: no <main id="main">')
        if not p.desc:
            err(f"{name}: no meta description")
        if name != "404.html" and p.current != 1:
            err(f"{name}: {p.current} current page keys")
        dupes = {i for i in p.ids if p.ids.count(i) > 1}
        if dupes:
            err(f"{name}: duplicate ids {sorted(dupes)}")

    root = SITE.resolve()

    def resolve(base: Path, ref: str, where: str) -> None:
        u = urlsplit(ref)
        if u.scheme in ("http", "https", "mailto"):
            return
        if u.scheme or ref.startswith("//"):
            err(f"{where}: unsupported URL {ref}")
            return
        if ref.startswith("/"):
            err(f"{where}: root-relative URL {ref}")
            return
        rel = (base.parent / unquote(u.path)).resolve() if u.path else base.resolve()
        if rel.is_relative_to(root) and rel.relative_to(root).as_posix().startswith(SKIP_PREFIXES):
            return
        if not rel.is_relative_to(root) or not rel.exists():
            err(f"{where}: missing {ref}")
            return
        if rel.is_dir():
            rel = rel / "index.html"
        if u.fragment and rel.suffix == ".html":
            p = parsed.get(rel.relative_to(root).as_posix())
            if p is not None and u.fragment not in p.ids:
                err(f"{where}: no #{u.fragment} in {rel.relative_to(root)}")

    for name, p in parsed.items():
        if p.base is not None and (name != "404.html" or p.base != BASE):
            err(f"{name}: unexpected <base href={p.base!r}>")
        for tag, ref in p.refs:
            if tag in ("img", "script", "link", "source") and urlsplit(ref).scheme in ("http", "https"):
                err(f"{name}: third-party asset {ref}")
            resolve(pages[name], ref, name)
    for css in SITE.rglob("*.css"):
        for ref in re.findall(r"url\(\s*['\"]?([^'\")]+)", css.read_text()):
            if not ref.startswith("data:"):
                resolve(css, ref, str(css.relative_to(SITE)))

    if errors:
        print("\n".join(f"check-site: {e}" for e in errors[:400]))
        print(f"check-site: {len(errors)} problem(s)")
        return 1
    real = sum(1 for p in parsed.values() if not p.redirect)
    print(f"check-site: ok ({real} pages, {len(parsed) - real} redirects, {len(files)} files, {total // 1024} KiB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
