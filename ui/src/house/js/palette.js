// Optional palette switch (needs palettes.css). Usage: import { setPalette } from "./palette.js"; setPalette("peach").
// "default" (or anything unknown) clears it. Dark theme only; theme-init.js reapplies the saved choice pre-paint.
const KNOWN = ["peach", "lavender"];

export function setPalette(p) {
  p = KNOWN.includes(p) ? p : "default";
  if (p === "default") delete document.documentElement.dataset.palette;
  else document.documentElement.dataset.palette = p;
  try { p === "default" ? localStorage.removeItem("palette") : localStorage.setItem("palette", p); } catch { /* storage blocked */ }
  return p;
}
