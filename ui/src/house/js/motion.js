// Fine-pointer cursor trail. Usage: const stop = initCursorTrail(el); (el: position: relative; overflow: hidden)
// Does nothing (returns a no-op) under prefers-reduced-motion or without a fine pointer. Dots are styled by .hs-trail-dot in components.css.
export function initCursorTrail(el, { max = 16, gap = 14 } = {}) {
  if (!el || !matchMedia("(pointer: fine)").matches || matchMedia("(prefers-reduced-motion: reduce)").matches) return () => {};
  let lx = -1e3, ly = -1e3;
  const move = (e) => {
    if (e.pointerType !== "mouse") return;
    const r = el.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
    if (Math.hypot(x - lx, y - ly) < gap) return;
    [lx, ly] = [x, y];
    const d = document.createElement("i");
    d.className = "hs-trail-dot";
    d.setAttribute("aria-hidden", "true");
    d.style.left = x + "px"; // CSSOM write, not an inline style attribute in markup (CSP-safe)
    d.style.top = y + "px";
    d.addEventListener("animationend", () => d.remove(), { once: true });
    el.append(d);
    while (el.querySelectorAll(".hs-trail-dot").length > max) el.querySelector(".hs-trail-dot").remove();
  };
  el.addEventListener("pointermove", move);
  return () => { el.removeEventListener("pointermove", move); el.querySelectorAll(".hs-trail-dot").forEach((d) => d.remove()); };
}
