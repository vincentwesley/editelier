// Usage: import { initTabs } from "./tabs.js"; initTabs(document.querySelector(".hs-tabs")); tabs need aria-controls -> tabpanel id.
export function initTabs(root) {
  const tabs = [...root.querySelectorAll('[role="tab"]')];
  if (!tabs.length) return;
  const select = (tab, focus) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      const p = document.getElementById(t.getAttribute("aria-controls"));
      if (p) p.hidden = !on;
    }
    if (focus) tab.focus();
  };
  select(tabs.find((t) => t.getAttribute("aria-selected") === "true") || tabs[0], false);
  for (const t of tabs) t.addEventListener("click", () => select(t, false));
  root.addEventListener("keydown", (e) => {
    const i = tabs.indexOf(document.activeElement);
    if (i < 0) return;
    const n = tabs.length;
    const next = { ArrowRight: (i + 1) % n, ArrowLeft: (i + n - 1) % n, Home: 0, End: n - 1 }[e.key];
    if (next === undefined) return;
    e.preventDefault();
    select(tabs[next], true);
  });
}
