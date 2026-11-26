# Spike: preview surface (D1)

Branch `m1-spike-preview`, 2026-11-25. Question: can a wgpu DX12 surface on a native child HWND sit over a region the WebView2 UI reserves, and hold up under resize, maximize, minimize, DPI and keyboard focus?

**Verdict: keep D1, option (a).** It works first time with no flicker workarounds. The fallback (egui/wgpu pane) isn't needed.

## What was built
- `src-tauri/` (Tauri 2 app crate, workspace member). Window 1600×1000, min 1280×720, title "Editelier", background `#11150e`, CSP mirrored from `ui/index.html`, `withGlobalTauri` (no npm dep needed for `invoke`/`listen`).
- `src-tauri/src/preview.rs`: registers class `EditelierPreview`, creates a `WS_CHILD | WS_CLIPSIBLINGS` HWND under the Tauri main window (hidden until the first rect), `WM_ERASEBKGND` swallowed. A render thread owns a DX12-only wgpu instance, a surface on that HWND (`create_surface_unsafe` with a raw Win32 handle), `PresentMode::Fifo` (vsync), frame latency 2. Test pattern: moving gradient, a white sweeping bar (tearing/drops), 16 frame-counter bit cells along the bottom, and a 16 px magenta probe square top-left.
- Command `preview_set_rect {x, y, w, h, dpr}` (physical px, parent-client coords, clamped server-side) → `SetWindowPos(HWND_TOP, SWP_NOACTIVATE | SWP_SHOWWINDOW)`; the render thread reconfigures the surface on size change.
- `ui/src/preview.ts` (Tauri only): inserts `#preview` (16:9) + a stats line into `<main>`, `ResizeObserver` on the box and the root, a scroll listener and a `matchMedia(resolution)` watcher (a DPI change with no CSS-size change doesn't fire `ResizeObserver`) send `getBoundingClientRect() × devicePixelRatio`, edges rounded so sizes never drift. Rust emits `preview-stats` every 500 ms (fps, frame ms, process CPU, size).
- `--measure`: scripted run (warm-up, steady, 28-step resize sweep, maximize, restore, minimize, restore + focus), prints a table and exits by itself (≈18 s). Screen-probes the magenta square through `GetPixel` on the screen DC, so "probe (255,0,255)" means our HWND is on top and its frames reach DWM.

## Measurements
i5-1335U / Iris Xe (driver 32.0.101.7088), 1920×1200 at **125 %** (DPI 120), 60 Hz, WebView2 158.0.4370.33, release build (`--features custom-protocol`). CPU % = this process only (all threads, % of one core); WebView2 renders in its own processes and isn't counted. `acq` = `get_current_texture` wait (where DX12 vsync back-pressure lands), `work` = uniform write + encode + submit + present call.

| phase | frames | fps | mean ms | p99 ms | max ms | acq ms | work ms | CPU % | rect updates | reconfigs | probe |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm-up 1.5 s | 84 | 60.4 | 16.56 | 39.02 | 39.02 | 15.23 | 1.19 | 12.5 | 0 | 1 | magenta |
| steady 3 s | 186 | 60.0 | 16.67 | 19.89 | 23.89 | 15.23 | 1.44 | 8.9 | 0 | 0 | magenta |
| resize sweep 3 s | 172 | 55.2 | 18.11 | 71.71 | 82.79 | 9.68 | 2.39 | 25.0 | 28 | 28 | magenta |
| maximized 2.5 s | 153 | 58.5 | 17.11 | 33.69 | 65.44 | 14.79 | 1.90 | 8.7 | 1 | 1 | magenta |
| restored 1.5 s | 95 | 58.8 | 17.02 | 65.29 | 65.29 | 14.23 | 2.11 | 8.3 | 1 | 1 | magenta |
| minimized 2 s | 5 | — | — | — | — | — | — | 2.3 | 0 | 0 | n/a (window hidden) |
| un-minimized 2.5 s | 150 | 60.2 | 16.62 | 19.27 | 42.83 | 14.75 | 1.77 | 12.5 | 0 | 0 | magenta |

- A second run after the clippy fixes reproduced it: steady 59.9 fps, p99 18.8 ms, CPU 8.3 %; resize sweep 53.6 fps, p99 75 ms; resize → rect mean 20.2 ms, max 27.1 ms; all probes magenta; exit code 0.
- Resize → rect latency (window `set_size` → ResizeObserver → `invoke` → `SetWindowPos`), 28 steps: mean 18.7 ms, max 43.1 ms, i.e. about 1–2 frames. Every resize produced exactly one rect update and one surface reconfigure.
- Reconfigure stalls cost one to three dropped vsyncs (p99 72 ms during the sweep); steady state misses fewer than 1 % of vsyncs.
- Minimized: the loop idles on `IsIconic(parent)`. Without that, a flip-model Present to a hidden window returns at once and would spin a core.
- Child z-order seen at runtime: `EditelierPreview > WRY_WEBVIEW`. WebView2 never re-stacked above us across all phases (`HWND_TOP` is reasserted on each rect update anyway).
- Focus/keyboard: focused HWND is `Chrome_WidgetWin_1` (WebView2) before and after a click posted to the preview HWND; an `F24` sent with `SendInput` reached the page's `keydown`. A child window's `DefWindowProc` never takes focus, so the webview keeps the keyboard.
- An earlier run without bringing the window forward probed white: the window opened behind the terminal. That was occlusion, not z-order; `--measure` now calls `set_focus` first.

## Issues and limits
- **Not tested: a real DPI change to 150 %.** This laptop runs at 125 %, and changing scaling is a system setting. The code path is there (`matchMedia` resolution watcher + Tauri's `WM_DPICHANGED` resize). Owner check: set Display → Scale to 150 %, run `target/release/editelier.exe --measure`, confirm the `window DPI 144` line and magenta probes.
- **Flicker during live drag-resize** can't be judged headlessly. The rect trails the window by 1–2 frames, so during a fast drag the viewer edge lags the layout by up to ~40 ms. Acceptable for an editor; if it shows, hide the HWND on `WM_ENTERSIZEMOVE` and show it on `WM_EXITSIZEMOVE`, or synchronise through DComp later.
- **Airspace (expected, D1):** HTML can never draw over the viewer. Popovers, menus and modal dialogs that overlap the viewer must either avoid its rect or the viewer hides (`ShowWindow(SW_HIDE)`) while they're open. Spike page scrolls (credits footer); the viewer then covers the header. The real layout must not scroll the viewer region.
- **Window size:** 1600×1000 logical is 2000×1250 physical at 125 %, larger than the 1920×1200 screen; maximized it fits. M1 should size the first window to the work area (or start maximized).
- CPU % counts our process only; WebView2 GPU/renderer processes aren't included. Steady ~9 % of one core with the UI stats event every 500 ms.
- Chromium logs `Failed to unregister class Chrome_WidgetWin_0` on exit; harmless WebView2 shutdown noise.
- Not covered by this spike (D1 lists them): 4K-proxy playback and zero-copy D3D11→wgpu import. They belong to the decode steps of M1; the surface side is proven here.

## How to rerun
```
cd ui && npm run build && cd ..
cargo build --release --features custom-protocol
target/release/editelier.exe --measure > measure.txt   # exits by itself, ≈18 s
```
Dev: `cargo tauri dev` (Vite on :5173).

## Versions and licences
Rust 1.100.0 (pinned in `rust-toolchain.toml`), tauri-cli 2.12.6, MSVC 14.44 / Windows 11 SDK.

| crate | version | licence |
|---|---|---|
| tauri | 2.12.7 | Apache-2.0 OR MIT |
| tauri-build | 2.7.6 | Apache-2.0 OR MIT |
| wry (via tauri) | 0.57.4 | Apache-2.0 OR MIT |
| wgpu (features `dx12`, `wgsl`) | 30.0.2 | MIT OR Apache-2.0 |
| windows | 0.62.2 (same major tauri uses) | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| pollster | 1.0.3 | Apache-2.0/MIT |
| serde | 1.x | MIT OR Apache-2.0 |

Whole lockfile (all targets): no GPL/AGPL. MPL-2.0 (file-level copyleft, unmodified, fine): cssparser, cssparser-macros, selectors, dtoa-short, option-ext (Tauri build/utils). Two crates offer `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; we take MIT. The app icon `src-tauri/icons/icon.ico` was generated from the studio mark during the spike; since D22 it comes from Editelier's own mark (`brand/`).
