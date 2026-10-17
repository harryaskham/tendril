// Tendril site: mark the current section in the rail, and fade anything that scrolls sideways.
(() => {
  const links = [...document.querySelectorAll(".rail a[href^='#']")];
  const heads = links.map((a) => document.getElementById(a.getAttribute("href").slice(1))).filter(Boolean);
  const mark = () => {
    let cur = null;
    for (const h of heads) if (h.getBoundingClientRect().top < window.innerHeight * 0.35) cur = h.id;
    links.forEach((a) => a.classList.toggle("on", a.getAttribute("href") === "#" + cur));
  };
  if (heads.length) { window.addEventListener("scroll", mark, { passive: true }); mark(); }

  const fade = (el) => {
    if (el.scrollWidth - el.clientWidth - el.scrollLeft > 4) el.setAttribute("data-scroll-more", "");
    else el.removeAttribute("data-scroll-more");
  };
  document.querySelectorAll("pre, .table, .fig-scroll").forEach((el) => {
    if (el.scrollWidth > el.clientWidth + 4) {
      el.tabIndex = 0;
      if (!el.hasAttribute("aria-label")) el.setAttribute("aria-label", "Scrollable " + (el.tagName === "PRE" ? "code" : el.classList.contains("table") ? "table" : "figure"));
      fade(el);
      el.addEventListener("scroll", () => fade(el), { passive: true });
    }
  });
})();
