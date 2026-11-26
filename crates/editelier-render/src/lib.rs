//! Editelier's wgpu renderer (D5, D20): imports decoded frames, converts tagged Y'CbCr to RGB,
//! runs the per-clip stage (input transform → inverse DRT → grade → DRT) into a display-linear
//! f16 clip texture, composites the canvas (fit/fill, titles) and writes outputs: Rec.709 code
//! values for the preview surface or an RGBA8 readback, and NV12 BT.709 limited for export.
//!
//! The device is the caller's: the preview creates one for its surface and shares it, export and
//! tests use [`Gpu::headless`].
//!
//! ponytail: decoded GPU frames reach wgpu through a CPU staging copy (D3D11 map → `write_texture`),
//! not a shared NT handle. Zero-copy (D3D11 shared texture → D3D12 resource → `create_texture_from_hal`
//! + fence) when 4K originals in preview or export throughput need it.

mod quality;
mod text;

pub use quality::ssim;
pub use text::Fonts;

use editelier_core::TitleClip;
use std::fmt;
use text::TitleCache;

#[derive(Debug)]
pub struct RenderError(pub String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

pub type Result<T> = std::result::Result<T, RenderError>;

/// A wgpu device + queue. Clones share the device.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Features the renderer uses when the adapter has them (P010 needs 16-bit norm textures).
    pub fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
        adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
    }

    /// DX12 device without a surface (export, tests, thumbnails).
    pub fn headless() -> Result<Gpu> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| RenderError(format!("no GPU adapter: {e}")))?;
        Self::from_adapter(&adapter, "render")
    }

    pub fn from_adapter(adapter: &wgpu::Adapter, label: &str) -> Result<Gpu> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: Self::wanted_features(adapter),
            ..Default::default()
        }))
        .map_err(|e| RenderError(format!("no GPU device: {e}")))?;
        Ok(Gpu { device, queue })
    }
}

/// Y'CbCr matrix of a source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl Matrix {
    fn kr_kb(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// How to read a source's Y'CbCr: matrix and range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceColor {
    pub matrix: Matrix,
    pub full_range: bool,
}

impl SourceColor {
    /// From FFmpeg tag names; untagged falls back to 601 for ≤ 576 lines, else 709, limited (D5).
    pub fn from_tags(matrix: Option<&str>, range: Option<&str>, height: u32) -> Self {
        let matrix = match matrix {
            Some("bt709") => Matrix::Bt709,
            Some("bt470bg" | "smpte170m" | "bt601") => Matrix::Bt601,
            Some(m) if m.starts_with("bt2020") => Matrix::Bt2020,
            _ if height <= 576 => Matrix::Bt601,
            _ => Matrix::Bt709,
        };
        SourceColor {
            matrix,
            full_range: matches!(range, Some("pc" | "full" | "jpeg")),
        }
    }
}

/// Decoded pixels. NV12/P010: `data` holds `height` Y rows then `height / 2` CbCr rows, each
/// `stride` bytes (the media crate's CPU frame layout). RGBA: straight alpha, R'G'B' code values.
#[derive(Clone, Copy, Debug)]
pub enum Pixels<'a> {
    Nv12 { data: &'a [u8], stride: usize },
    P010 { data: &'a [u8], stride: usize },
    Rgba8 { data: &'a [u8], stride: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct SourceFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels<'a>,
    pub color: SourceColor,
}

/// How a clip sits in a canvas of another aspect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fit {
    /// Whole frame visible, letterbox or pillarbox bars.
    #[default]
    Contain,
    /// Canvas filled, edges cropped.
    Cover,
}

const SHADERS: &str = include_str!("shaders.wgsl");
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn shader(device: &wgpu::Device, section: &str) -> wgpu::ShaderModule {
    let common = SHADERS.split("\n//#").next().unwrap_or_default();
    let body = SHADERS
        .split("\n//#")
        .find(|s| s.starts_with(section))
        .unwrap_or_default();
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(section),
        source: wgpu::ShaderSource::Wgsl(format!("{common}\n//{body}").into()),
    })
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn uniform(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("uniform"),
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// QuadU: rect, tint, mode.
fn quad_uniform(rect: [f32; 4], tint: [f32; 4], mode: u32) -> Vec<u8> {
    let mut b = floats(&rect);
    b.extend(floats(&tint));
    b.extend(words(&[mode, 0, 0, 0]));
    b
}

/// NDC rect (x0, y0 bottom, x1, y1 top) of a `src`-sized image placed in `dst` with `fit`.
pub fn fit_rect(src: (u32, u32), dst: (u32, u32), fit: Fit) -> [f32; 4] {
    let (sw, sh) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (dw, dh) = (dst.0.max(1) as f32, dst.1.max(1) as f32);
    let s = match fit {
        Fit::Contain => (dw / sw).min(dh / sh),
        Fit::Cover => (dw / sw).max(dh / sh),
    };
    let (hx, hy) = (sw * s / dw, sh * s / dh);
    [-hx, -hy, hx, hy]
}

struct Clip {
    size: (u32, u32),
    kind: u8,
    a: wgpu::Texture,
    b: wgpu::Texture,
    out: wgpu::Texture,
}

/// One renderer per output (preview or an export job). Holds the current clip frame, the canvas
/// and cached title masks.
pub struct Renderer {
    gpu: Gpu,
    clip_pipe: wgpu::RenderPipeline,
    quad_pipe: wgpu::RenderPipeline,
    out_pipes: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    nv12_pipe: wgpu::ComputePipeline,
    quad_module: wgpu::ShaderModule,
    linear: wgpu::Sampler,
    dummy: wgpu::Texture,
    clip: Option<Clip>,
    show_clip: bool,
    canvas: Option<wgpu::Texture>,
    titles: TitleCache,
    pub fit: Fit,
}

impl Renderer {
    pub fn new(gpu: &Gpu, fonts: Fonts) -> Self {
        let device = &gpu.device;
        let clip_mod = shader(device, "clip");
        let quad_module = shader(device, "quad");
        let nv_mod = shader(device, "nv12");
        let target = |format, blend| {
            [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };
        let render = |m: &wgpu::ShaderModule, vs: &str, fs: &str, targets: &[Option<_>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: None,
                vertex: wgpu::VertexState {
                    module: m,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: m,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let clip_pipe = render(
            &clip_mod,
            "vs_full",
            "fs_clip",
            &target(CANVAS_FORMAT, None),
        );
        let quad_pipe = render(
            &quad_module,
            "vs_quad",
            "fs_quad",
            &target(
                CANVAS_FORMAT,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
        );
        let nv12_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("nv12"),
            layout: None,
            module: &nv_mod,
            entry_point: Some("cs_nv12"),
            compilation_options: Default::default(),
            cache: None,
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = texture(
            device,
            "dummy",
            (1, 1),
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        Renderer {
            gpu: gpu.clone(),
            clip_pipe,
            quad_pipe,
            out_pipes: Vec::new(),
            nv12_pipe,
            quad_module,
            linear,
            dummy,
            clip: None,
            show_clip: false,
            canvas: None,
            titles: TitleCache::new(fonts),
            fit: Fit::Contain,
        }
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    fn view(t: &wgpu::Texture) -> wgpu::TextureView {
        t.create_view(&Default::default())
    }

    /// Upload a decoded frame and run the per-clip stage. Replaces the shown clip frame.
    pub fn upload(&mut self, f: &SourceFrame) -> Result<()> {
        let (w, h) = (f.width, f.height);
        if w == 0 || h == 0 {
            return Err(RenderError("empty frame".into()));
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (kind, fa, fb, code_scale, data, stride) = match f.pixels {
            Pixels::Nv12 { data, stride } => (
                0u8,
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureFormat::Rg8Unorm,
                255.0,
                data,
                stride,
            ),
            Pixels::P010 { data, stride } => {
                if !self
                    .gpu
                    .device
                    .features()
                    .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
                {
                    return Err(RenderError(
                        "this GPU can't sample 10-bit video textures".into(),
                    ));
                }
                (
                    1,
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                    65535.0 / 256.0,
                    data,
                    stride,
                )
            }
            Pixels::Rgba8 { data, stride } => (
                2,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba8Unorm,
                255.0,
                data,
                stride,
            ),
        };
        let bpp = fa.block_copy_size(None).unwrap_or(1) as usize;
        let rows = if kind == 2 {
            h as usize
        } else {
            (h + ch) as usize
        };
        if stride < w as usize * bpp || data.len() < stride * (rows - 1) + w as usize * bpp {
            return Err(RenderError("frame buffer is smaller than its size".into()));
        }
        let device = &self.gpu.device;
        if !self
            .clip
            .as_ref()
            .is_some_and(|c| c.size == (w, h) && c.kind == kind)
        {
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
            let b_size = if kind == 2 { (1, 1) } else { (cw, ch) };
            self.clip = Some(Clip {
                size: (w, h),
                kind,
                a: texture(device, "clip-a", (w, h), fa, usage),
                b: texture(device, "clip-b", b_size, fb, usage),
                out: texture(
                    device,
                    "clip-out",
                    (w, h),
                    CANVAS_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                ),
            });
        }
        let clip = self.clip.as_ref().expect("clip set above");
        let write = |t: &wgpu::Texture, offset: usize, size: (u32, u32)| {
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data[offset..],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
        };
        write(&clip.a, 0, (w, h));
        if kind != 2 {
            write(&clip.b, stride * h as usize, (cw, ch));
        }
        let (kr, kb) = f.color.matrix.kr_kb();
        let mut u = floats(&[kr, kb, code_scale, f.color.full_range as u32 as f32]);
        u.extend(words(&[(kind == 2) as u32, 0, 0, 0]));
        let ubuf = uniform(device, &u);
        let b_view = if kind == 2 {
            Self::view(&self.dummy)
        } else {
            Self::view(&clip.b)
        };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clip"),
            layout: &self.clip_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&Self::view(&clip.a)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let out = Self::view(&clip.out);
            let mut pass = begin(&mut enc, &out, Some(wgpu::Color::TRANSPARENT));
            pass.set_pipeline(&self.clip_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.gpu.queue.submit([enc.finish()]);
        self.show_clip = true;
        Ok(())
    }

    /// Show no clip (black canvas under the titles) until the next [`upload`](Self::upload).
    pub fn clear_clip(&mut self) {
        self.show_clip = false;
    }

    /// Composite the canvas: black, the current clip fitted, then `titles` (positions in canvas
    /// units, sizes in project pixels times `title_scale`).
    pub fn compose(&mut self, size: (u32, u32), titles: &[&TitleClip], title_scale: f32) {
        let device = self.gpu.device.clone();
        if !self.canvas.as_ref().is_some_and(|c| {
            let s = c.size();
            (s.width, s.height) == size
        }) {
            self.canvas = Some(texture(
                &device,
                "canvas",
                size,
                CANVAS_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ));
        }
        let mut quads: Vec<(wgpu::TextureView, Vec<u8>)> = Vec::new();
        if self.show_clip
            && let Some(c) = &self.clip
        {
            let rect = fit_rect(c.size, size, self.fit);
            quads.push((Self::view(&c.out), quad_uniform(rect, [0.0; 4], 0)));
        }
        for t in titles {
            if let Some((tex, rect)) =
                self.titles
                    .get(&device, &self.gpu.queue, t, size, title_scale)
            {
                let [r, g, b, a] = t.style.color.map(|c| c as f32 / 255.0);
                let lin = |c: f32| c.powf(2.4) * a;
                quads.push((tex, quad_uniform(rect, [lin(r), lin(g), lin(b), a], 1)));
            }
        }
        let binds: Vec<wgpu::BindGroup> = quads
            .iter()
            .map(|(view, u)| self.quad_bind(&self.quad_pipe, view, u))
            .collect();
        let canvas = Self::view(self.canvas.as_ref().expect("canvas set above"));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut enc, &canvas, Some(wgpu::Color::BLACK));
            pass.set_pipeline(&self.quad_pipe);
            for b in &binds {
                pass.set_bind_group(0, b, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.gpu.queue.submit([enc.finish()]);
    }

    fn quad_bind(
        &self,
        pipe: &wgpu::RenderPipeline,
        view: &wgpu::TextureView,
        u: &[u8],
    ) -> wgpu::BindGroup {
        let ubuf = uniform(&self.gpu.device, u);
        self.gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("quad"),
                layout: &pipe.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: ubuf.as_entire_binding(),
                    },
                ],
            })
    }

    fn out_pipe(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some((_, p)) = self.out_pipes.iter().find(|(f, _)| *f == format) {
            return p.clone();
        }
        let p = self
            .gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("out"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &self.quad_module,
                    entry_point: Some("vs_quad"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.quad_module,
                    entry_point: Some("fs_out"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        self.out_pipes.push((format, p.clone()));
        p
    }

    /// Record the output transform of the canvas into `target` (fitted, black bars) on `enc`.
    pub fn encode_output(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) {
        let Some(canvas) = &self.canvas else {
            drop(begin(enc, target, Some(wgpu::Color::BLACK)));
            return;
        };
        let s = canvas.size();
        let rect = fit_rect((s.width, s.height), size, Fit::Contain);
        let view = Self::view(canvas);
        let pipe = self.out_pipe(format);
        let bind = self.quad_bind(
            &pipe,
            &view,
            &quad_uniform(rect, [0.0; 4], format.is_srgb() as u32),
        );
        let mut pass = begin(enc, target, Some(wgpu::Color::BLACK));
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..6, 0..1);
    }

    fn canvas_size(&self) -> Result<(u32, u32)> {
        let c = self
            .canvas
            .as_ref()
            .ok_or_else(|| RenderError("nothing composed yet".into()))?;
        Ok((c.size().width, c.size().height))
    }

    /// The canvas as tightly packed RGBA8 Rec.709 code values (tests, thumbnails).
    pub fn read_rgba8(&mut self) -> Result<Vec<u8>> {
        let (w, h) = self.canvas_size()?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let t = texture(
            &self.gpu.device,
            "readback",
            (w, h),
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        self.encode_output(&mut enc, &Self::view(&t), format, (w, h));
        enc.copy_texture_to_buffer(
            t.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            t.size(),
        );
        self.gpu.queue.submit([enc.finish()]);
        let bytes = self.map(&buf)?;
        Ok(bytes
            .chunks(row as usize)
            .flat_map(|r| &r[..w as usize * 4])
            .copied()
            .collect())
    }

    /// The canvas as tightly packed NV12 (Y then interleaved CbCr), BT.709 limited range.
    /// The canvas width must be a multiple of 4 and the height even.
    pub fn read_nv12(&mut self) -> Result<Vec<u8>> {
        let (w, h) = self.canvas_size()?;
        if !w.is_multiple_of(4) || !h.is_multiple_of(2) {
            return Err(RenderError(format!(
                "export size {w}x{h}: width must be a multiple of 4 and height even"
            )));
        }
        let len = (w * h * 3 / 2) as u64;
        let device = &self.gpu.device;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12"),
            size: len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12-read"),
            size: len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let ubuf = uniform(device, &words(&[w, h, 0, 0]));
        let canvas = Self::view(self.canvas.as_ref().expect("checked above"));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12"),
            layout: &self.nv12_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&canvas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.nv12_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((w / 4).div_ceil(8), (h / 2).div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&out, 0, &read, 0, len);
        self.gpu.queue.submit([enc.finish()]);
        self.map(&read)
    }

    fn map(&self, buf: &wgpu::Buffer) -> Result<Vec<u8>> {
        buf.map_async(wgpu::MapMode::Read, .., |_| {});
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError(format!("GPU readback failed: {e}")))?;
        let bytes = buf
            .get_mapped_range(..)
            .map_err(|e| RenderError(format!("GPU readback failed: {e}")))?
            .to_vec();
        buf.unmap();
        Ok(bytes)
    }
}

fn begin<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: Option<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterbox_and_pillarbox() {
        assert_eq!(
            fit_rect((1920, 1080), (1920, 1080), Fit::Contain),
            [-1.0, -1.0, 1.0, 1.0]
        );
        // 16:9 into 9:16: full width, a third... of the height.
        let r = fit_rect((1920, 1080), (1080, 1920), Fit::Contain);
        assert!((r[2] - 1.0).abs() < 1e-6 && (r[3] - 0.31640625).abs() < 1e-6);
        // 9:16 into 16:9: pillarbox.
        let r = fit_rect((1080, 1920), (1920, 1080), Fit::Contain);
        assert!((r[3] - 1.0).abs() < 1e-6 && r[2] < 0.33);
        let r = fit_rect((1080, 1920), (1920, 1080), Fit::Cover);
        assert!((r[2] - 1.0).abs() < 1e-6 && r[3] > 3.0);
    }

    #[test]
    fn colour_tags() {
        let c = SourceColor::from_tags(None, None, 480);
        assert_eq!((c.matrix, c.full_range), (Matrix::Bt601, false));
        let c = SourceColor::from_tags(Some("bt709"), Some("pc"), 480);
        assert_eq!((c.matrix, c.full_range), (Matrix::Bt709, true));
        assert_eq!(
            SourceColor::from_tags(Some("bt2020nc"), None, 2160).matrix,
            Matrix::Bt2020
        );
    }
}
