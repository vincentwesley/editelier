// Usage: import { toast } from "./toast.js"; toast("Saved", { kind: "ok", ms: 4000 }); kind: info|ok|warn|danger
export function toast(message, { kind = "info", ms = 4000 } = {}) {
  let box = document.querySelector(".hs-toasts");
  if (!box) {
    box = document.createElement("div");
    box.className = "hs-toasts";
    box.setAttribute("role", "status");
    box.setAttribute("aria-live", "polite");
    document.body.append(box);
  }
  const el = document.createElement("div");
  el.className = "hs-toast hs-toast--" + kind;
  el.textContent = message;
  box.append(el);
  const done = () => el.remove();
  setTimeout(() => {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return done();
    el.setAttribute("data-leaving", "");
    el.addEventListener("animationend", done, { once: true });
    setTimeout(done, 600); // fallback if animationend never fires
  }, ms);
}
