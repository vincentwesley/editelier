import "./style.css";
import "./house/credits/credits.js";
import "./house/brand/logo/logo-resize";
import { initDialogs } from "./dialogs";
import { initEditor } from "./editor";
import { applyTheme, nextTheme, type Theme } from "./theme";
import { initWelcome } from "./welcome";

const KEY = "theme";
const store = {
  get: (): Theme => {
    try {
      return localStorage.getItem(KEY) === "light" ? "light" : "dark";
    } catch {
      return "dark";
    }
  },
  set: (t: Theme) => {
    try {
      localStorage.setItem(KEY, t);
    } catch {
      /* storage blocked: theme just won't persist */
    }
  },
};

const toggles = document.querySelectorAll<HTMLButtonElement>("[data-theme-toggle]");
function sync(t: Theme): void {
  applyTheme(t);
  for (const btn of toggles) {
    btn.setAttribute("aria-pressed", String(t === "light"));
    btn.textContent = t === "light" ? "Dark theme" : "Light theme";
  }
}
sync(store.get());
for (const btn of toggles)
  btn.addEventListener("click", () => {
    const t = nextTheme(store.get());
    store.set(t);
    sync(t);
  });

initDialogs();
initEditor();
initWelcome();
