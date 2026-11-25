//! M1 spike (D1): wgpu (DX12) test pattern rendered into a native WS_CHILD HWND that sits
//! above the WebView2 controller, over a rect the UI reserves and reports.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_TOP, IsIconic, RegisterClassW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetWindowPos, WINDOW_EX_STYLE, WM_ERASEBKGND, WNDCLASSW, WS_CHILD,
    WS_CLIPSIBLINGS,
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
    /// Uniform write + encode + submit + present call.
    pub cpu_ms: Vec<f64>,
    pub reconfigures: u32,
    pub surface_errors: u32,
}

impl Window {
    pub fn merge(&mut self, o: Window) {
        self.intervals_ms.extend(o.intervals_ms);
        self.acquire_ms.extend(o.acquire_ms);
        self.cpu_ms.extend(o.cpu_ms);
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

    pub fn set_rect(&self, r: Rect) {
        let r = r.sanitized();
        *self.shared.rect.lock().unwrap() = r;
        self.shared.rect_updates.fetch_add(1, Ordering::Relaxed);
        // HWND_TOP keeps us above the WebView2 sibling; never activate (focus stays in the webview).
        let _ = unsafe {
            SetWindowPos(
                HWND(self.hwnd as _),
                Some(HWND_TOP),
                r.x,
                r.y,
                r.w as i32,
                r.h as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
    }

    pub fn start(&mut self) {
        let (hwnd, parent, shared) = (self.hwnd, self.parent, self.shared.clone());
        self.thread = Some(
            std::thread::Builder::new()
                .name("preview-render".into())
                .spawn(move || {
                    if let Err(e) = render_loop(hwnd, parent, &shared) {
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

const SHADER: &str = r#"
struct U { size: vec2<f32>, time: f32, frame: u32 };
@group(0) @binding(0) var<uniform> u: U;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Probe square: solid magenta in the top-left 16 px, sampled from the screen by --measure.
    if (pos.x < 16.0 && pos.y < 16.0) { return vec4<f32>(1.0, 0.0, 1.0, 1.0); }
    let uv = pos.xy / u.size;
    // Frame-counter bars: 16 cells along the bottom, cell i lit when bit i of the frame is set.
    if (uv.y > 0.92) {
        let bit = u32(uv.x * 16.0);
        let on = (u.frame >> bit) & 1u;
        return select(vec4<f32>(0.05, 0.05, 0.05, 1.0), vec4<f32>(0.31, 0.88, 0.63, 1.0), on == 1u);
    }
    // Moving vertical bar exposes tearing and dropped frames.
    let bar = abs(fract(u.time * 0.25) - uv.x) < 0.004;
    if (bar) { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }
    let g = fract(uv.x + u.time * 0.2);
    return vec4<f32>(g, uv.y, 0.5 + 0.5 * sin(u.time), 1.0);
}
"#;

fn render_loop(hwnd: isize, parent: isize, shared: &Shared) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = unsafe {
        let mut wh = raw_window_handle::Win32WindowHandle::new(
            std::num::NonZeroIsize::new(hwnd).ok_or("null hwnd")?,
        );
        wh.hinstance = None;
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_window_handle::RawDisplayHandle::Windows(
                raw_window_handle::WindowsDisplayHandle::new(),
            )),
            raw_window_handle: raw_window_handle::RawWindowHandle::Win32(wh),
        })
    }
    .map_err(|e| e.to_string())?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;
    eprintln!("preview: adapter {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("preview"),
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
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

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pattern"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("u"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pattern"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: ubuf.as_entire_binding(),
        }],
    });

    let start = Instant::now();
    let mut last_present: Option<Instant> = None;
    let mut frame: u32 = 0;
    while !shared.stop.load(Ordering::Relaxed) {
        let r = *shared.rect.lock().unwrap();
        // Minimized/empty: a flip-model Present to an occluded window returns at once
        // (no vsync wait) and would spin a core, so idle instead.
        if r.w == 0 || r.h == 0 || unsafe { IsIconic(HWND(parent as _)) }.as_bool() {
            last_present = None;
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if (r.w, r.h) != (config.width, config.height) {
            config.width = r.w;
            config.height = r.h;
            surface.configure(&device, &config);
            shared.frames.lock().unwrap().reconfigures += 1;
        }
        let t0 = Instant::now();
        let tex = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                shared.frames.lock().unwrap().surface_errors += 1;
                if !matches!(other, wgpu::CurrentSurfaceTexture::Timeout) {
                    surface.configure(&device, &config);
                }
                continue;
            }
        };
        let acquire = t0.elapsed();
        let t1 = Instant::now();
        let view = tex.texture.create_view(&Default::default());
        let mut u = [0u8; 16];
        u[0..4].copy_from_slice(&(r.w as f32).to_le_bytes());
        u[4..8].copy_from_slice(&(r.h as f32).to_le_bytes());
        u[8..12].copy_from_slice(&start.elapsed().as_secs_f32().to_le_bytes());
        u[12..16].copy_from_slice(&frame.to_le_bytes());
        queue.write_buffer(&ubuf, 0, &u);
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
        queue.present(tex);
        let cpu = t1.elapsed();
        frame = frame.wrapping_add(1);
        let now = Instant::now();
        let mut w = shared.frames.lock().unwrap();
        if let Some(p) = last_present {
            w.intervals_ms.push((now - p).as_secs_f64() * 1e3);
        }
        w.acquire_ms.push(acquire.as_secs_f64() * 1e3);
        w.cpu_ms.push(cpu.as_secs_f64() * 1e3);
        last_present = Some(now);
    }
    Ok(())
}
