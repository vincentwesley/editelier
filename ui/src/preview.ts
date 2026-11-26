// The viewer: report the #preview box's physical-pixel rect to Rust, which positions a native wgpu child HWND over it (D1).
// HTML can never draw over that HWND (airspace), so while a modal or popover is open the viewer is sent a zero rect.
// No-op outside Tauri.

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
}

/** CSS-pixel rect → physical pixels, rounding edges (not sizes) so adjacent rects never gap. */
function toPhysical(r: { left: number; top: number; right: number; bottom: number }, dpr: number) {
  const x = Math.round(r.left * dpr);
  const y = Math.round(r.top * dpr);
  return { x, y, w: Math.max(0, Math.round(r.right * dpr) - x), h: Math.max(0, Math.round(r.bottom * dpr) - y) };
}

export function mountPreview(box: HTMLElement): void {
  const tauri = (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
  if (!tauri) return;
  let last = "";
  const send = () => {
    const dpr = window.devicePixelRatio;
    const covered = !box.isConnected || box.closest("[hidden]") || document.querySelector(":modal, :popover-open");
    const rect = covered ? { x: 0, y: 0, w: 0, h: 0, dpr } : { ...toPhysical(box.getBoundingClientRect(), dpr), dpr };
    const key = JSON.stringify(rect);
    if (key === last) return;
    last = key;
    void tauri.core.invoke("preview_set_rect", { rect });
  };
  new ResizeObserver(send).observe(box);
  new ResizeObserver(send).observe(document.documentElement);
  window.addEventListener("scroll", send, { passive: true });
  // Dialogs and popovers fire `toggle`; it doesn't bubble, so listen in the capture phase.
  document.addEventListener("toggle", () => requestAnimationFrame(send), true);
  document.addEventListener("editelier:layout", send);
  // DPI change without a CSS-size change doesn't fire ResizeObserver; watch the resolution.
  const watchDpr = () =>
    matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`).addEventListener(
      "change",
      () => {
        send();
        watchDpr();
      },
      { once: true },
    );
  watchDpr();
  send();
  // Measure mode sends F24 to check keyboard input still reaches the webview.
  window.addEventListener("keydown", (e) => {
    if (e.key === "F24") void tauri.core.invoke("ui_key", { key: e.key });
  });
}
