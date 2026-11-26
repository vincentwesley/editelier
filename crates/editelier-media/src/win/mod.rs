//! Windows implementation: Media Foundation decode/encode (H.264, HEVC, AAC and whatever MF has),
//! D3D11 frames, WIC stills.

pub mod mf;
pub mod wic;

pub use mf::{VideoDecoder, decode_audio, make_proxy, proxy_path};

use crate::Error;
use std::sync::Once;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Media::MediaFoundation::{IMFSample, MF_VERSION, MFSTARTUP_FULL, MFStartup};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows_core::Interface;

pub(crate) fn err(context: &str) -> impl Fn(windows_core::Error) -> Error + '_ {
    move |e| Error::Codec(format!("{context}: {e}"))
}

/// COM (MTA) on this thread + MFStartup once per process. Never shut down: MF is ref-counted and
/// lives for the app's lifetime.
pub(crate) fn init() -> crate::Result<()> {
    thread_local!(static COM: () = unsafe {
        // S_FALSE / RPC_E_CHANGED_MODE are fine: COM is usable either way.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    });
    COM.with(|_| ());
    static MF: Once = Once::new();
    let mut res = Ok(());
    MF.call_once(|| {
        res = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(err("MFStartup"))
    });
    res
}

/// Creates a hardware D3D11 device suitable for MF decode (video support, multithread protected).
/// The render crate will normally create its own and pass it in instead.
pub fn create_device() -> crate::Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
        .map_err(err("D3D11CreateDevice"))?;
    }
    let device = device.ok_or_else(|| Error::Codec("no D3D11 device".into()))?;
    protect(&device)?;
    Ok(device)
}

/// MF requires multithread protection on a device shared with the decoder.
pub(crate) fn protect(device: &ID3D11Device) -> crate::Result<()> {
    let mt: ID3D11Multithread = device.cast().map_err(err("ID3D11Multithread"))?;
    unsafe {
        let _ = mt.SetMultithreadProtected(true);
    }
    Ok(())
}

/// A decoded frame still on the GPU: `texture` is often a texture array; use `subresource`.
/// Import into wgpu via a copy into a shared (NT handle) texture in the render crate.
pub struct GpuFrame {
    pub texture: ID3D11Texture2D,
    pub subresource: u32,
    // Holds the decoder surface out of MF's pool while the frame is alive.
    pub(crate) _sample: IMFSample,
}
