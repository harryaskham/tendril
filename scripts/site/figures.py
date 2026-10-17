"""Inline SVG figures for the Tendril site.

Authored near the width they render (700 for the text column, 900 for the home figure) and
styled by classes in site/assets/site.css. Colour roles: target (windows, displays,
cameras, devices), shot (captures), act (input sent with run), lock (the execution lock),
agent (MCP and agents), far (remote, WSL, Android) and mute.

Motif: `strip` draws a strip of film with sprocket holes; the loop is a sequence of frames.

FIGURES maps a name to a function returning one <svg>.
"""

from __future__ import annotations

from html import escape

ROLES = ("target", "shot", "act", "lock", "agent", "far", "mute")
W_BOLD = 8.1   # Schibsted Grotesk 700 14.5px
W_BODY = 6.7   # Schibsted Grotesk 13px
W_MONO = 7.2   # Fragment Mono 12px


class Svg:
    def __init__(self, name: str, w: int, h: int, title: str, desc: str):
        self.name, self.w, self.h, self.title, self.desc = name, w, h, title, desc
        self.parts: list[str] = []

    def add(self, s: str) -> None:
        self.parts.append(s)

    def text(self, x, y, s, cls="f-s", anchor="start") -> None:
        a = "" if anchor == "start" else f' text-anchor="{anchor}"'
        if s.startswith(" ") or "  " in s:
            a += ' xml:space="preserve"'
        self.add(f'<text x="{x}" y="{y}" class="{cls}"{a}>{escape(s)}</text>')

    def rect(self, x, y, w, h, cls, r=6) -> None:
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" class="{cls}"/>')

    def strip(self, x, y, w, h) -> None:
        """A strip of film: dark base with sprocket holes along both edges."""
        self.rect(x, y, w, h, "frame", 2)
        for i in range(int(w // 22)):
            hx = x + 8 + i * 22
            self.rect(hx, y + 5, 10, 7, "hole", 2)
            self.rect(hx, y + h - 12, 10, 7, "hole", 2)

    def wire(self, pts, role="ink", arrow=True, dashed=False) -> None:
        d = "M" + " L".join(f"{x} {y}" for x, y in pts)
        m = f' marker-end="url(#{self.name}-a-{role})"' if arrow else ""
        self.add(f'<path d="{d}" class="wire s-{role}{" dash" if dashed else ""}"{m}/>')

    def curve(self, x1, y1, x2, y2, role="ink", arrow=True, dashed=False) -> None:
        mx = (x1 + x2) / 2
        m = f' marker-end="url(#{self.name}-a-{role})"' if arrow else ""
        self.add(f'<path d="M{x1} {y1} C{mx} {y1} {mx} {y2} {x2} {y2}" class="wire s-{role}{" dash" if dashed else ""}"{m}/>')

    def box(self, x, y, w, h, role, title, sub=(), mono=False) -> None:
        self.rect(x, y, w, h, f"b-{role}", 8)
        lines = [sub] if isinstance(sub, str) else list(sub)
        total = 16 + 16 * len(lines)
        ty = y + (h - total) / 2 + 13
        self.text(x + 12, ty, title, "f-t")
        for i, s in enumerate(lines):
            self.text(x + 12, ty + 18 + 16 * i, s, "f-m" if mono else "f-s")

    def label(self, x, y, s, cls="f-xs", anchor="middle", per=6.4) -> None:
        w = len(s) * per + 10
        x0 = x - w / 2 if anchor == "middle" else (x - 5 if anchor == "start" else x - w + 5)
        self.rect(round(x0, 1), y - 13, round(w, 1), 18, "labbg", 3)
        self.text(x, y, s, cls, anchor)

    def svg(self) -> str:
        n = self.name
        marks = "".join(
            f'<marker id="{n}-a-{r}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
            f'orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" class="m-{r}"/></marker>' for r in ROLES)
        return (f'<svg class="f" viewBox="0 0 {self.w} {self.h}" width="{self.w}" height="{self.h}" role="img" '
                f'aria-labelledby="{n}-t {n}-d"><title id="{n}-t">{escape(self.title)}</title>'
                f'<desc id="{n}-d">{escape(self.desc)}</desc><defs>{marks}</defs>\n' + "\n".join(self.parts) + "\n</svg>")


# ============================================================================ home

def hero() -> str:
    s = Svg(
        "hero", 900, 330, "The loop on a strip of film",
        "On the left, an agent, either through the command line or the MCP server. On the right, a strip of film with "
        "four frames. Frame one, list: discover windows, displays and cameras. Frame two, capture: take a screenshot "
        "and its coordinate scale. Frame three, act: run typed input, clicks and scrolls, holding the execution lock "
        "and restoring focus afterwards. Frame four, verify: capture again and compare. An arrow returns from frame "
        "four to frame two when the result is not yet right. Below the strip, the places a frame can come from: X11, "
        "Wayland, macOS, Windows, Android over ADB, any of them over SSH, and the Windows host from WSL.",
    )
    s.box(0, 70, 150, 110, "agent", "agent", ("CLI or MCP", "stateless calls"))
    s.wire([(152, 125), (196, 125)], "agent")
    s.strip(200, 20, 700, 210)
    frames = [("01", "list", "tendril list", "target"), ("02", "capture", "… capture", "shot"), ("03", "act", "… run '…'", "act"),
              ("04", "verify", "… capture", "shot")]
    for i, (n, t, cmd, role) in enumerate(frames):
        x = 214 + i * 172
        s.rect(x, 40, 160, 170, f"b-{role}", 3)
        s.text(x + 10, 60, n, "f-xs")
        s.text(x + 80, 110, t, "f-d", "middle")
        s.text(x + 80, 136, cmd, "f-m", "middle")
    s.text(638, 172, "lock held", "f-xs", "middle")
    s.text(638, 190, "focus restored", "f-xs", "middle")
    s.add('<path d="M800 212 C800 262 380 262 380 212" class="wire s-act dash" marker-end="url(#hero-a-act)"/>')
    s.label(590, 258, "not right yet: capture again", "f-xs", per=6.9)
    places = ["X11", "Wayland", "macOS", "Windows", "Android", "SSH", "WSL"]
    x = 200
    for p in places:
        w = len(p) * 8.6 + 22
        s.rect(x, 290, w, 28, "b-far" if p in ("Android", "SSH", "WSL") else "b-target", 14)
        s.text(x + w / 2, 309, p, "f-t", "middle")
        x += w + 10
    return s.svg()


# ============================================================================ guide

def scale() -> str:
    s = Svg(
        "scale", 700, 306, "Capture space and source space",
        "On the left, the window's source space: 797 by 587 pixels, the space run coordinates use, with the Save "
        "button at 72, 354. On the right, the capture an agent sees, scaled down to 512 by 377. The same button "
        "appears at about 46, 227 in the capture. The capture reports both ratios: source to output, 512 over 797 by "
        "377 over 587, and output to source, 797 over 512 by 587 over 377. Multiply a point in the capture by output "
        "to source to get the coordinates to click.",
    )
    s.rect(0, 30, 318, 234, "b-target", 3)
    s.text(10, 22, "source space · 797 × 587", "f-xs")
    s.rect(14, 160, 40, 16, "b-act", 3)
    s.text(60, 173, "Save (72, 354)", "f-m")
    s.rect(420, 70, 204, 151, "b-shot", 3)
    s.text(420, 44, "what the agent sees", "f-xs")
    s.text(420, 62, "capture · 512 × 377", "f-xs")
    s.rect(429, 156, 26, 10, "b-act", 2)
    s.text(462, 166, "(46, 227)", "f-m")
    s.wire([(452, 172), (452, 250), (60, 250), (60, 180)], "act")
    s.text(256, 274, "× output_to_source = 797/512 × 587/377", "f-m", "middle")
    s.text(0, 298, "run takes source coordinates; captures may be smaller", "f-xs")
    return s.svg()


# ============================================================================ agents

def lockq() -> str:
    s = Svg(
        "lockq", 700, 250, "The execution lock",
        "Three run commands from different agents arrive at once. Each writes a queue ticket and waits its turn. The "
        "first ticket takes the lock: it writes an owner record and refreshes a heartbeat while its input runs, then "
        "releases. The next ticket then proceeds. A holder whose heartbeat stops for 30 seconds is reaped as stale, "
        "and a waiter gives up after 60 seconds by default. Reads such as list and capture do not take the lock.",
    )
    s.text(0, 18, "queue/", "f-xs")
    for i, who in enumerate(["agent A", "agent B", "MCP run"]):
        y = 30 + i * 52
        s.rect(0, y, 150, 40, "b-plain", 4)
        s.text(12, y + 25, f"ticket · {who}", "f-m")
    s.wire([(152, 50), (246, 100)], "lock")
    s.rect(250, 70, 200, 74, "b-lock", 4)
    s.text(264, 94, "held/owner.json", "f-m")
    s.text(264, 114, "heartbeat while running", "f-s")
    s.text(264, 132, "then release", "f-s")
    s.wire([(452, 107), (540, 107)], "act")
    s.box(544, 76, 156, 62, "act", "input runs", "one run at a time")
    s.wire([(152, 102), (246, 112)], "mute", dashed=True)
    s.wire([(152, 154), (246, 124)], "mute", dashed=True)
    s.text(250, 176, "stale holder reaped after 30 s without a heartbeat", "f-xs")
    s.text(250, 196, "a waiter times out after 60 s (both configurable)", "f-xs")
    s.text(250, 216, "list and capture never wait for the lock", "f-xs")
    return s.svg()


# ============================================================================ platforms

def routes() -> str:
    s = Svg(
        "routes", 700, 300, "Same invocation, different places",
        "A local tendril invocation can run in four places. Without flags it drives the local desktop. With --remote "
        "user at host it strips that flag and runs the same tendril command over SSH on another machine, streaming "
        "the result back. With --wsl-tunnel, from WSL or Linux, it runs the Windows build, tendril.exe, on the "
        "Windows host. With --android and a serial it drives an Android device or emulator over ADB.",
    )
    s.box(0, 112, 170, 76, "agent", "tendril …", ("same arguments", "same envelopes"))
    rows = [(20, "target", "local desktop", "X11, Wayland, macOS, Windows", ""), (90, "far", "--remote user@host", "OpenSSH → tendril on that host", ""),
            (160, "far", "--wsl-tunnel", "→ tendril.exe on the Windows host", ""), (230, "far", "--android SERIAL", "ADB → device or emulator", "")]
    for y, role, t, sub, _ in rows:
        s.curve(172, 150, 330, y + 25, role)
        s.box(334, y, 366, 52, role, t, sub)
    return s.svg()


FIGURES = {
    "hero": hero,
    "scale": scale,
    "lockq": lockq,
    "routes": routes,
}
