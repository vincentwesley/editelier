// House toast with an optional fix button and a 44px close button. Errors stay until closed; others time out.
import type { ApiError } from "./api/types";
import { h } from "./dom";

interface ToastOpts {
  kind?: "info" | "ok" | "warn" | "danger";
  action?: { label: string; run: () => void };
  ms?: number;
}

export function toast(message: string, { kind = "info", action, ms = kind === "danger" ? 0 : 5000 }: ToastOpts = {}): HTMLElement {
  let box = document.querySelector<HTMLElement>(".hs-toasts");
  if (!box) {
    box = h("div", { class: "hs-toasts", role: "status", "aria-live": "polite" });
    document.body.append(box);
  }
  const close = () => el.remove();
  const el = h(
    "div",
    { class: `hs-toast hs-toast--${kind} toast` },
    h("p", { class: "toast__msg" }, message),
    action ? h("button", { type: "button", class: "hs-btn hs-btn--secondary hs-btn--sm", onclick: () => (close(), action.run()) }, action.label) : null,
    h("button", { type: "button", class: "toast__close", "aria-label": "Dismiss", onclick: close }, h("span", { class: "hs-icon i-close", "aria-hidden": "true" })),
  );
  box.append(el);
  if (ms) setTimeout(close, ms);
  return el;
}

/** Plain-language messages with one fix, keyed by the core's error codes (research 03 §6). */
const FRIENDLY: Record<string, string> = {
  media_offline: "This clip moved or was renamed.",
  unsupported_codec: "This file uses a format Editelier can't open yet.",
  hevc_extension_missing: "This clip needs the free HEVC Video Extensions from the Microsoft Store.",
  disk_full: "The disk is full. Free some space, then try again.",
  empty_timeline: "Add a clip to the timeline before exporting.",
};

export function showError(err: unknown, retry?: () => void): void {
  const e = err as Partial<ApiError> | undefined;
  const msg = (e?.code && FRIENDLY[e.code]) || e?.message || "Something went wrong. Your project is safe.";
  const fix =
    e?.code === "hevc_extension_missing"
      ? { label: "Open the Store", run: () => window.open("ms-windows-store://pdp/?ProductId=9NMZLZ57R3T7") }
      : retry
        ? { label: "Try again", run: retry }
        : undefined;
  toast(msg, { kind: "danger", action: fix });
}
