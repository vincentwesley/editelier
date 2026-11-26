//! Zero-copy D3D11 <-> wgpu (DX12) frame exchange on one adapter (D3).
//!
//! Decoded frames: the decoder's surface (often a texture-array slice) is copied GPU-side into a
//! pooled shared NV12 texture (NT handle), which wgpu samples as a native NV12 texture. Export
//! output: the renderer writes NV12 into a pooled shared texture through plane wrappers, and the
//! encoder side copies it into its own sample. No pixel crosses the CPU.
//!
//! Sync: shared fence `A` is signalled by D3D11 (a write or read of a slot is done) and waited
//! on by the D3D12 queue (GPU side); fence `B` is signalled by D3D12 and waited on by D3D11 users
//! on the CPU (a GPU wait would stall the D3D11 context MF shares). Each slot remembers the value
//! after the other API's last use, so a slot is rewritten only once the GPU is done with it.
//! Fence values are taken and signalled under one lock per side so they stay monotonic.

use crate::{Gpu, RenderError, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wgpu::hal::api::Dx12;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext4,
    ID3D11Fence, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_FENCE_FLAG_SHARED, ID3D12Device, ID3D12Fence, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIAdapter,
    IDXGIFactory1, IDXGIResource1,
};
use windows_core::{Interface, PCWSTR};

fn e(context: &'static str) -> impl Fn(windows_core::Error) -> RenderError {
    move |err| RenderError(format!("{context}: {err}"))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// D3D11 writes, wgpu samples (decoded frames).
    In,
    /// wgpu writes, D3D11 reads (export output).
    Out,
}

struct Slot {
    kind: Kind,
    size: (u32, u32),
    tex: ID3D11Texture2D,
    /// `In`: the NV12 texture. `Out`: the Y plane wrapper.
    a: wgpu::Texture,
    /// `Out`: the CbCr plane wrapper.
    b: Option<wgpu::Texture>,
    /// Value of the other side's fence after its last use of this slot.
    done: AtomicU64,
}

struct Shared {
    gpu: Gpu,
    d3d11: ID3D11Device,
    d12: ID3D12Device,
    a11: ID3D11Fence,
    a12: ID3D12Fence,
    b12: ID3D12Fence,
    /// Last signalled value of A (D3D11 side lock) and B (D3D12 side lock).
    a: Mutex<u64>,
    b: Mutex<u64>,
    pool: Mutex<Vec<Arc<Slot>>>,
    /// Total time the render side blocked on D3D11 work (diagnostics).
    waited_us: AtomicU64,
}

/// Cheap to clone; one per GPU. Create with [`Interop::new`].
#[derive(Clone)]
pub struct Interop(Arc<Shared>);

/// A decoded frame in a shared slot, ready for [`Renderer::upload`](crate::Renderer::upload)
/// once the D3D12 queue waits on `ready`.
#[derive(Clone)]
pub struct GpuImage {
    interop: Interop,
    slot: Arc<Slot>,
    ready: u64,
}

/// One rendered NV12 output frame in a shared slot, from
/// [`Renderer::render_nv12`](crate::Renderer::render_nv12).
pub struct Nv12Frame {
    interop: Interop,
    slot: Arc<Slot>,
    ready: u64,
}

impl std::fmt::Debug for GpuImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuImage({:?}, ready {})", self.slot.size, self.ready)
    }
}

impl GpuImage {
    pub fn size(&self) -> (u32, u32) {
        self.slot.size
    }
}

fn pack_luid(high: i32, low: u32) -> u64 {
    ((high as u32 as u64) << 32) | low as u64
}

impl Interop {
    /// A D3D11 device (video support, multithread protected) on `gpu`'s adapter, sharing
    /// textures and fences with it. Fails (callers fall back to CPU copies) when the adapter
    /// lacks NV12 textures, D3D11.4 fences or shared NT handles, or `EDITELIER_ZERO_COPY=0`
    /// (A/B measurements).
    pub fn new(gpu: &Gpu) -> Result<Self> {
        if std::env::var_os("EDITELIER_ZERO_COPY").is_some_and(|v| v == "0") {
            return Err(RenderError("EDITELIER_ZERO_COPY=0".into()));
        }
        if !gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
        {
            return Err(RenderError("no NV12 texture support".into()));
        }
        let d12 = unsafe { gpu.device.as_hal::<Dx12>() }
            .ok_or_else(|| RenderError("not a DX12 device".into()))?
            .raw_device()
            .clone();
        let luid = unsafe { d12.GetAdapterLuid() };
        let want = pack_luid(luid.HighPart, luid.LowPart);
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(e("DXGI factory"))?;
            let adapter: IDXGIAdapter = (0..)
                .map_while(|i| factory.EnumAdapters1(i).ok())
                .find(|a| {
                    a.GetDesc1().is_ok_and(|d| {
                        pack_luid(d.AdapterLuid.HighPart, d.AdapterLuid.LowPart) == want
                    })
                })
                .ok_or_else(|| RenderError("wgpu adapter not found in DXGI".into()))?
                .cast()
                .map_err(e("adapter"))?;
            let mut device = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
            .map_err(e("D3D11CreateDevice"))?;
            let d3d11: ID3D11Device =
                device.ok_or_else(|| RenderError("no D3D11 device".into()))?;
            let _ = d3d11
                .cast::<ID3D11Multithread>()
                .map_err(e("multithread"))?
                .SetMultithreadProtected(true);
            let d5: ID3D11Device5 = d3d11.cast().map_err(e("D3D11.4 fences"))?;
            let fence = || -> Result<(ID3D11Fence, ID3D12Fence)> {
                let f12: ID3D12Fence = d12
                    .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                    .map_err(e("fence"))?;
                let h = d12
                    .CreateSharedHandle(&f12, None, GENERIC_ALL.0, PCWSTR::null())
                    .map_err(e("fence handle"))?;
                let mut f11: Option<ID3D11Fence> = None;
                let r = d5.OpenSharedFence(h, &mut f11);
                let _ = CloseHandle(h);
                r.map_err(e("open fence"))?;
                Ok((f11.ok_or_else(|| RenderError("no fence".into()))?, f12))
            };
            let (a11, a12) = fence()?;
            let b12: ID3D12Fence = d12
                .CreateFence(
                    0,
                    windows::Win32::Graphics::Direct3D12::D3D12_FENCE_FLAG_NONE,
                )
                .map_err(e("fence"))?;
            Ok(Interop(Arc::new(Shared {
                gpu: gpu.clone(),
                d3d11,
                d12,
                a11,
                a12,
                b12,
                a: Mutex::new(0),
                b: Mutex::new(0),
                pool: Mutex::new(Vec::new()),
                waited_us: AtomicU64::new(0),
            })))
        }
    }

    /// The D3D11 device to decode and encode on.
    pub fn device(&self) -> &ID3D11Device {
        &self.0.d3d11
    }

    /// Seconds the render side has spent waiting for D3D11 copies (diagnostics).
    pub fn waited_seconds(&self) -> f64 {
        self.0.waited_us.load(Ordering::Relaxed) as f64 / 1e6
    }

    /// The shared adapter's LUID (high part << 32 | low part).
    pub fn adapter_luid(&self) -> u64 {
        let l = unsafe { self.0.d12.GetAdapterLuid() };
        pack_luid(l.HighPart, l.LowPart)
    }

    fn slot(&self, kind: Kind, size: (u32, u32)) -> Result<Arc<Slot>> {
        let mut pool = self.0.pool.lock().unwrap();
        if let Some(s) = pool
            .iter()
            .find(|s| s.kind == kind && s.size == size && Arc::strong_count(s) == 1)
        {
            return Ok(s.clone());
        }
        // Drop idle slots of other sizes before growing.
        pool.retain(|s| Arc::strong_count(s) > 1 || s.kind != kind || s.size == size);
        let s = Arc::new(self.create_slot(kind, size)?);
        pool.push(s.clone());
        Ok(s)
    }

    fn create_slot(&self, kind: Kind, (w, h): (u32, u32)) -> Result<Slot> {
        let sh = &self.0;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0)
                as u32,
        };
        let (tex, res) = unsafe {
            let mut t = None;
            sh.d3d11
                .CreateTexture2D(&desc, None, Some(&mut t))
                .map_err(e("shared texture"))?;
            let tex: ID3D11Texture2D = t.ok_or_else(|| RenderError("no texture".into()))?;
            let h: HANDLE = tex
                .cast::<IDXGIResource1>()
                .map_err(e("IDXGIResource1"))?
                .CreateSharedHandle(
                    None,
                    DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
                    PCWSTR::null(),
                )
                .map_err(e("texture handle"))?;
            let mut r: Option<ID3D12Resource> = None;
            let ok = sh.d12.OpenSharedHandle(h, &mut r);
            let _ = CloseHandle(h);
            ok.map_err(e("open texture"))?;
            (
                tex,
                r.ok_or_else(|| RenderError("no D3D12 resource".into()))?,
            )
        };
        let wrap = |format, size: (u32, u32), plane: Option<u32>, usage| {
            let extent = wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            };
            let mut hal = unsafe {
                wgpu::hal::dx12::Device::texture_from_raw(
                    res.clone(),
                    format,
                    wgpu::TextureDimension::D2,
                    extent,
                    1,
                    1,
                )
            };
            if let Some(p) = plane {
                hal = hal.with_plane_slice(p);
            }
            unsafe {
                sh.gpu.device.create_texture_from_hal::<Dx12>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("shared"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            }
        };
        let (a, b) = match kind {
            Kind::In => (
                wrap(
                    wgpu::TextureFormat::NV12,
                    (w, h),
                    None,
                    wgpu::TextureUsages::TEXTURE_BINDING,
                ),
                None,
            ),
            Kind::Out => (
                wrap(
                    wgpu::TextureFormat::R8Unorm,
                    (w, h),
                    Some(0),
                    wgpu::TextureUsages::COPY_DST,
                ),
                Some(wrap(
                    wgpu::TextureFormat::Rg8Unorm,
                    (w / 2, h / 2),
                    Some(1),
                    wgpu::TextureUsages::COPY_DST,
                )),
            ),
        };
        Ok(Slot {
            kind,
            size: (w, h),
            tex,
            a,
            b,
            done: AtomicU64::new(0),
        })
    }

    /// Runs `f` on the D3D11 immediate context after it waits for B >= `wait_b`, then signals
    /// A (recorded as the last D3D11 use of an output slot).
    fn d3d11_use(
        &self,
        slot: &Slot,
        wait_b: u64,
        f: impl FnOnce(&ID3D11DeviceContext4),
    ) -> Result<u64> {
        let sh = &self.0;
        let mt: ID3D11Multithread = sh.d3d11.cast().map_err(e("multithread"))?;
        let ctx: ID3D11DeviceContext4 = unsafe { sh.d3d11.GetImmediateContext() }
            .map_err(e("context"))?
            .cast()
            .map_err(e("context4"))?;
        // CPU wait, not `ctx.Wait`: a GPU wait would stall the whole D3D11 context, which the MF
        // decoder and encoder share, and convoy the D3D12 queue behind it.
        if unsafe { sh.b12.GetCompletedValue() } < wait_b {
            unsafe { sh.b12.SetEventOnCompletion(wait_b, HANDLE::default()) }
                .map_err(e("fence wait"))?;
        }
        let mut a = sh.a.lock().unwrap();
        unsafe {
            mt.Enter();
            f(&ctx);
            let r = ctx.Signal(&sh.a11, *a + 1);
            ctx.Flush();
            mt.Leave();
            r.map_err(e("D3D11 fence"))?;
        }
        *a += 1;
        if slot.kind == Kind::Out {
            slot.done.store(*a, Ordering::Release);
        }
        Ok(*a)
    }

    /// Copies a decoded NV12 surface (`subresource` of `src`, visible `size`) into a shared slot.
    pub fn import(
        &self,
        src: &ID3D11Texture2D,
        subresource: u32,
        size: (u32, u32),
    ) -> Result<GpuImage> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { src.GetDesc(&mut desc) };
        let (w, h) = (
            (size.0.min(desc.Width) & !1).max(2),
            (size.1.min(desc.Height) & !1).max(2),
        );
        let slot = self.slot(Kind::In, (w, h))?;
        let src_box = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: w,
            bottom: h,
            back: 1,
        };
        let wait = slot.done.load(Ordering::Acquire);
        let ready = self.d3d11_use(&slot, wait, |ctx| unsafe {
            ctx.CopySubresourceRegion(&slot.tex, 0, 0, 0, 0, src, subresource, Some(&src_box));
        })?;
        Ok(GpuImage {
            interop: self.clone(),
            slot,
            ready,
        })
    }

    /// Copies an output frame into `dst` (an NV12 texture of the same size on [`device`]
    /// (Self::device), `subresource`), GPU-side; the slot is reusable once that copy ran.
    pub fn copy_out(&self, f: &Nv12Frame, dst: &ID3D11Texture2D, subresource: u32) -> Result<()> {
        self.d3d11_use(&f.slot, f.ready, |ctx| unsafe {
            ctx.CopySubresourceRegion(dst, subresource, 0, 0, 0, &f.slot.tex, 0, None);
        })
        .map(|_| ())
    }

    /// Blocks until D3D11's signal `value` (call before the submit). A CPU wait: a queue-side
    /// `Wait` on the cross-device fence stalled later submits for 50-300 ms on Iris Xe. With
    /// decode-ahead and pooled slots the value has almost always passed already.
    fn queue_wait(&self, value: u64) -> Result<()> {
        if unsafe { self.0.a12.GetCompletedValue() } < value {
            let t = std::time::Instant::now();
            unsafe { self.0.a12.SetEventOnCompletion(value, HANDLE::default()) }
                .map_err(e("fence wait"))?;
            let us = t.elapsed().as_micros() as u64;
            self.0.waited_us.fetch_add(us, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Signals B after everything submitted so far (recorded as the last D3D12 use of an input
    /// slot).
    fn queue_signal(&self, slot: &Slot) -> Result<u64> {
        let q = unsafe { self.0.gpu.queue.as_hal::<Dx12>() }
            .ok_or_else(|| RenderError("not a DX12 queue".into()))?;
        let mut b = self.0.b.lock().unwrap();
        unsafe { q.as_raw().Signal(&self.0.b12, *b + 1) }.map_err(e("queue signal"))?;
        *b += 1;
        if slot.kind == Kind::In {
            slot.done.store(*b, Ordering::Release);
        }
        Ok(*b)
    }
}

impl GpuImage {
    /// NV12 plane views (Y, CbCr) for sampling.
    pub(crate) fn views(&self) -> (wgpu::TextureView, wgpu::TextureView) {
        let view = |format, aspect| {
            self.slot.a.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                aspect,
                ..Default::default()
            })
        };
        (
            view(wgpu::TextureFormat::R8Unorm, wgpu::TextureAspect::Plane0),
            view(wgpu::TextureFormat::Rg8Unorm, wgpu::TextureAspect::Plane1),
        )
    }

    /// Submit `cmd` on the D3D12 queue after the import copy, then mark the slot read.
    pub(crate) fn submit(&self, cmd: wgpu::CommandBuffer) -> Result<()> {
        let i = &self.interop;
        i.queue_wait(self.ready)?;
        i.0.gpu.queue.submit([cmd]);
        i.queue_signal(&self.slot).map(|_| ())
    }
}

impl Interop {
    /// A free output slot of `size` and its plane wrappers (Y, CbCr).
    pub(crate) fn out_slot(&self, size: (u32, u32)) -> Result<OutSlot> {
        Ok(OutSlot(self.slot(Kind::Out, size)?))
    }
}

pub(crate) struct OutSlot(Arc<Slot>);

impl OutSlot {
    pub(crate) fn planes(&self) -> (&wgpu::Texture, &wgpu::Texture) {
        (
            &self.0.a,
            self.0.b.as_ref().expect("out slots have two planes"),
        )
    }

    /// Submit `cmd` (which writes the planes) once D3D11 is done reading the slot.
    pub(crate) fn submit(self, interop: &Interop, cmd: wgpu::CommandBuffer) -> Result<Nv12Frame> {
        interop.queue_wait(self.0.done.load(Ordering::Acquire))?;
        interop.0.gpu.queue.submit([cmd]);
        let ready = interop.queue_signal(&self.0)?;
        // Nothing else polls during an export: let wgpu free finished per-frame resources.
        let _ = interop.0.gpu.device.poll(wgpu::PollType::Poll);
        Ok(Nv12Frame {
            interop: interop.clone(),
            slot: self.0,
            ready,
        })
    }
}

impl Nv12Frame {
    pub fn interop(&self) -> &Interop {
        &self.interop
    }
}
