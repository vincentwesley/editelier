//! The viewer (D1): a native WS_CHILD HWND above the WebView2 controller, over the rect the UI
//! reserves and reports, presenting the renderer's canvas with wgpu (DX12) on the shared device.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::Engine;
use editelier_core::{Id, Layer, TitleClip};
use editelier_render::{Fonts, Gpu, Renderer};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_TOP, IsIconic, RegisterClassW, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, WINDOW_EX_STYLE, WM_ERASEBKGND, WNDCLASSW,
    WS_CHILD, WS_CLIPSIBLINGS,
};
use windows::core::w;

/// Physical-pixel rect relative to the parent client area, as sent by the UI.
#[derive(Clone, Copy, Default, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub dpr: f64,
}

impl Rect {
    /// Clamp untrusted UI input to sane bounds.
    pub fn sanitized(self) -> Self {
        Self {
            x: self.x.clamp(-32_768, 32_768),
            y: self.y.clamp(-32_768, 32_768),
            w: self.w.min(16_384),
            h: self.h.min(16_384),
            dpr: if self.dpr.is_finite() {
                self.dpr.clamp(0.25, 8.0)
            } else {
                1.0
            },
        }
    }
}

/// One frame-timing sample window, drained by whoever reads stats.
#[derive(Default)]
pub struct Window {
    pub intervals_ms: Vec<f64>,
    /// get_current_texture wait (where the vsync back-pressure lands on DX12).
    pub acquire_ms: Vec<f64>,
    /// Compose + encode + submit + present call.
    pub cpu_ms: Vec<f64>,
    /// New video pictures shown (content frame rate) and their upload + clip-pass cost.
    pub uploads: u32,
    pub upload_ms: Vec<f64>,
    pub reconfigures: u32,
    pub surface_errors: u32,
}

impl Window {
    pub fn merge(&mut self, o: Window) {
        self.intervals_ms.extend(o.intervals_ms);
        self.acquire_ms.extend(o.acquire_ms);
        self.cpu_ms.extend(o.cpu_ms);
        self.uploads += o.uploads;
        self.upload_ms.extend(o.upload_ms);
        self.reconfigures += o.reconfigures;
        self.surface_errors += o.surface_errors;
    }
}

pub struct Shared {
    pub rect: Mutex<Rect>,
    pub rect_updates: AtomicU64,
    pub stop: AtomicBool,
    pub frames: Mutex<Window>,
}

pub struct Preview {
    pub hwnd: isize,
    pub parent: isize,
    pub shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_ERASEBKGND {
        return LRESULT(1); // wgpu owns every pixel; skip GDI fill (avoids flashes on resize)
    }
    unsafe { DefWindowProcW(h, m, w, l) }
}

impl Preview {
    /// Must run on the thread that owns `parent` (Tauri main thread).
    pub fn create(parent: isize) -> windows::core::Result<Self> {
        let hwnd = unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: inst.into(),
                lpszClassName: w!("EditelierPreview"),
                ..Default::default()
            };
            RegisterClassW(&wc);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EditelierPreview"),
                w!("Preview"),
                WS_CHILD | WS_CLIPSIBLINGS, // hidden until the UI reports a rect
                0,
                0,
                0,
                0,
                Some(HWND(parent as _)),
                None,
                Some(inst.into()),
                None,
            )?
        };
        Ok(Self {
            hwnd: hwnd.0 as isize,
            parent,
            shared: Arc::new(Shared {
                rect: Mutex::new(Rect::default()),
                rect_updates: AtomicU64::new(0),
                stop: AtomicBool::new(false),
                frames: Mutex::new(Window::default()),
            }),
            thread: None,
        })
    }

    /// Place the viewer; a zero-size rect hides it (modals, popovers, other screens).
    pub fn set_rect(&self, r: Rect) {
        let r = r.sanitized();
        *self.shared.rect.lock().unwrap() = r;
        self.shared.rect_updates.fetch_add(1, Ordering::Relaxed);
        let show = if r.w == 0 || r.h == 0 {
            SWP_HIDEWINDOW
        } else {
            SWP_SHOWWINDOW
        };
        // HWND_TOP keeps us above the WebView2 sibling; never activate (focus stays in the webview).
        let _ = unsafe {
            SetWindowPos(
                HWND(self.hwnd as _),
                Some(HWND_TOP),
                r.x,
                r.y,
                r.w as i32,
                r.h as i32,
                SWP_NOACTIVATE | show,
            )
        };
    }

    pub fn start(&mut self, gpu: Gpu, engine: Arc<Engine>) {
        let (hwnd, parent, shared) = (self.hwnd, self.parent, self.shared.clone());
        self.thread = Some(
            std::thread::Builder::new()
                .name("preview-render".into())
                .spawn(move || {
                    if let Err(e) = render_loop(hwnd, parent, &shared, &gpu, &engine) {
                        eprintln!("preview: {e}");
                    }
                })
                .expect("spawn render thread"),
        );
    }

    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Preview canvas: the project size, at most 1920 on the long edge (4K projects preview at
/// 1080p-class resolution; export renders full size).
fn canvas_size(w: u32, h: u32) -> ((u32, u32), f32) {
    let s = (1920.0 / w.max(h).max(1) as f32).min(1.0);
    let even = |v: f32| ((v.round() as u32) & !1).max(2);
    ((even(w as f32 * s), even(h as f32 * s)), s)
}

fn render_loop(
    hwnd: isize,
    parent: isize,
    shared: &Shared,
    gpu: &Gpu,
    engine: &Engine,
) -> Result<(), String> {
    let surface = unsafe {
        let mut wh = raw_window_handle::Win32WindowHandle::new(
            std::num::NonZeroIsize::new(hwnd).ok_or("null hwnd")?,
        );
        wh.hinstance = None;
        gpu.instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_window_handle::RawDisplayHandle::Windows(
                    raw_window_handle::WindowsDisplayHandle::new(),
                )),
                raw_window_handle: raw_window_handle::RawWindowHandle::Win32(wh),
            })
    }
    .map_err(|e| e.to_string())?;
    eprintln!("preview: adapter {:?}", gpu.adapter.get_info().name);
    let caps = surface.get_capabilities(&gpu.adapter);
    // A UNORM target gets the Rec.709 code values as-is (what a player shows for the export).
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: 0,
        height: 0,
        present_mode: wgpu::PresentMode::Fifo, // vsync
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        color_space: Default::default(),
    };
    let mut renderer = Renderer::new(gpu, Fonts::bundled());
    let mut shown: Option<(Id, (i64, bool))> = None;
    let mut last_present: Option<Instant> = None;
    while !shared.stop.load(Ordering::Relaxed) {
        let r = *shared.rect.lock().unwrap();
        // Hidden/minimized: a flip-model Present to an occluded window returns at once
        // (no vsync wait) and would spin a core, so idle instead.
        if r.w == 0 || r.h == 0 || unsafe { IsIconic(HWND(parent as _)) }.as_bool() {
            last_present = None;
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if (r.w, r.h) != (config.width, config.height) {
            config.width = r.w;
            config.height = r.h;
            surface.configure(&gpu.device, &config);
            shared.frames.lock().unwrap().reconfigures += 1;
        }
        let t0 = Instant::now();
        let tex = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                shared.frames.lock().unwrap().surface_errors += 1;
                if !matches!(other, wgpu::CurrentSurfaceTexture::Timeout) {
                    surface.configure(&gpu.device, &config);
                }
                continue;
            }
        };
        let acquire = t0.elapsed();
        let t1 = Instant::now();

        // Picture for the clock time.
        let (t, hit) = engine.display();
        let mut upload = None;
        if let Some((_, pic)) = hit {
            let key = pic.as_ref().map(|(m, f)| (*m, f.key));
            if key != shown {
                let u0 = Instant::now();
                match &pic {
                    Some((_, f)) => {
                        if let Err(e) = renderer.upload(&f.source()) {
                            eprintln!("preview: {e}");
                        }
                    }
                    None => renderer.clear_clip(),
                }
                shown = key;
                upload = Some(u0.elapsed());
            }
        }
        let p = engine.project();
        let layers = p.layers_at(t);
        let titles: Vec<&TitleClip> = layers
            .iter()
            .filter_map(|l| match l {
                Layer::Title(t) => Some(*t),
                _ => None,
            })
            .collect();
        let (size, scale) = canvas_size(p.settings.width, p.settings.height);
        renderer.compose(size, &titles, scale);
        let view = tex.texture.create_view(&Default::default());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        renderer.encode_output(&mut enc, &view, format, (r.w, r.h));
        gpu.queue.submit([enc.finish()]);
        gpu.queue.present(tex);
        let cpu = t1.elapsed();
        let now = Instant::now();
        let mut w = shared.frames.lock().unwrap();
        if let Some(p) = last_present {
            w.intervals_ms.push((now - p).as_secs_f64() * 1e3);
        }
        if let Some(u) = upload {
            w.uploads += 1;
            w.upload_ms.push(u.as_secs_f64() * 1e3);
        }
        w.acquire_ms.push(acquire.as_secs_f64() * 1e3);
        w.cpu_ms.push(cpu.as_secs_f64() * 1e3);
        last_present = Some(now);
    }
    Ok(())
}
