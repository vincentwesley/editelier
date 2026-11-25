// M1 spike hook: reserve a #preview region and report its physical-pixel rect to Rust,
// which positions a native wgpu child HWND over it (D1). No-op outside Tauri.

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: { listen: <T>(name: string, cb: (e: { payload: T }) => void) => Promise<() => void> };
}
interface PreviewStats {
  fps: number;
  frame_ms: number;
  cpu: number;
  w: number;
  h: number;
}

/** CSS-pixel rect → physical pixels, rounding edges (not sizes) so adjacent rects never gap. */
function toPhysical(r: { left: number; top: number; right: number; bottom: number }, dpr: number) {
  const x = Math.round(r.left * dpr);
  const y = Math.round(r.top * dpr);
  return { x, y, w: Math.max(0, Math.round(r.right * dpr) - x), h: Math.max(0, Math.round(r.bottom * dpr) - y) };
}

const tauri = (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
if (tauri) {
  document.body.classList.add("has-preview");
  const box = document.createElement("div");
  box.id = "preview";
  box.setAttribute("role", "img");
  box.setAttribute("aria-label", "Preview");
  const stats = document.createElement("p");
  stats.id = "preview-stats";
  stats.className = "preview-stats";
  document.querySelector("main")!.prepend(box, stats);

  let last = "";
  const send = () => {
    const dpr = window.devicePixelRatio;
    const rect = { ...toPhysical(box.getBoundingClientRect(), dpr), dpr };
    const key = JSON.stringify(rect);
    if (key === last) return;
    last = key;
    void tauri.core.invoke("preview_set_rect", { rect });
  };
  new ResizeObserver(send).observe(box);
  new ResizeObserver(send).observe(document.documentElement);
  window.addEventListener("scroll", send, { passive: true });
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

  void tauri.event.listen<PreviewStats>("preview-stats", ({ payload: s }) => {
    stats.textContent = `${s.fps.toFixed(1)} fps · ${s.frame_ms.toFixed(2)} ms · CPU ${s.cpu.toFixed(1)}% · ${s.w}×${s.h}`;
  });
  // Measure mode sends F24 to check keyboard input still reaches the webview.
  window.addEventListener("keydown", (e) => {
    if (e.key === "F24") void tauri.core.invoke("ui_key", { key: e.key });
  });
}
