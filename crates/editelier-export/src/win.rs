//! Media Foundation encoder enumeration and the MP4 [`Exporter`] (IMFSinkWriter).
//!
//! Only OS/vendor MFTs encode H.264/HEVC/AAC here; no FFmpeg encoder is involved (D3).

use crate::{
    AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, CancelToken, EncoderInfo, ExportError, Preset, Progress,
    RateControl, Vendor, VideoCodec, aac_bytes_per_second, rgba_to_nv12,
};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{GUID, HSTRING, Interface, PWSTR};

impl From<windows::core::Error> for ExportError {
    fn from(e: windows::core::Error) -> Self {
        ExportError::Media(format!("{:#010x} {}", e.code().0, e.message()))
    }
}

type Result<T> = std::result::Result<T, ExportError>;

fn startup() -> Result<()> {
    // COM init is per thread and ref-counted; RPC_E_CHANGED_MODE (thread already STA) is fine.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
    Ok(())
}

fn subtype(codec: VideoCodec) -> GUID {
    match codec {
        VideoCodec::H264 => MFVideoFormat_H264,
        VideoCodec::Hevc => MFVideoFormat_HEVC,
    }
}

fn get_string(attrs: &IMFAttributes, key: &GUID) -> Option<String> {
    let mut p = PWSTR::null();
    let mut len = 0u32;
    unsafe {
        attrs.GetAllocatedString(key, &mut p, &mut len).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

fn get_luid(attrs: &IMFAttributes) -> Option<u64> {
    unsafe {
        if let Ok(v) = attrs.GetUINT64(&MFT_ENUM_ADAPTER_LUID) {
            return Some(v);
        }
        let mut b = [0u8; 8];
        attrs
            .GetBlob(&MFT_ENUM_ADAPTER_LUID, &mut b, None)
            .ok()
            .map(|_| u64::from_le_bytes(b))
    }
}

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

/// Hardware encoder MFTs for `codec` (sorted Intel, AMD, NVIDIA, other) followed by
/// software MFTs (the Microsoft H.264 encoder; HEVC only with the Store extension).
pub fn enumerate_encoders(codec: VideoCodec) -> Result<Vec<EncoderInfo>> {
    startup()?;
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype(codec),
    };
    let mut out = Vec::new();
    for (flags, hardware) in [
        (MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER, true),
        (MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER, false),
    ] {
        let mut ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                None,
                Some(&output),
                &mut ptr,
                &mut n,
            )?;
            if ptr.is_null() {
                continue;
            }
            for slot in std::slice::from_raw_parts_mut(ptr, n as usize) {
                let Some(act) = slot.take() else { continue };
                let name = get_string(&act, &MFT_FRIENDLY_NAME_Attribute)
                    .unwrap_or_else(|| "unnamed encoder".into());
                let vendor_id = get_string(&act, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute);
                out.push(EncoderInfo {
                    vendor: Vendor::identify(vendor_id.as_deref(), &name, hardware),
                    name,
                    codec,
                    hardware,
                    adapter_luid: if hardware { get_luid(&act) } else { None },
                });
            }
            CoTaskMemFree(Some(ptr as _));
        }
    }
    out.sort_by_key(|e| e.vendor); // stable: keeps MF merit order within a vendor
    // Drivers can register the same encoder twice (seen with Intel QSV); MF picks one.
    out.dedup();
    Ok(out)
}

/// D3D11 device (video support, multithread-protected) on the adapter with `luid`,
/// or the default adapter, wrapped in a DXGI device manager.
pub fn create_device_manager(luid: Option<u64>) -> Result<IMFDXGIDeviceManager> {
    startup()?;
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let adapter: Option<IDXGIAdapter> = luid.and_then(|want| {
            (0..)
                .map_while(|i| factory.EnumAdapters1(i).ok())
                .find(|a| {
                    a.GetDesc1().is_ok_and(|d| {
                        pack(d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart) == want
                    })
                })
                .and_then(|a| a.cast().ok())
        });
        let driver = if adapter.is_some() {
            D3D_DRIVER_TYPE_UNKNOWN
        } else {
            D3D_DRIVER_TYPE_HARDWARE
        };
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            adapter.as_ref(),
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
        let device = device.ok_or_else(|| ExportError::Media("no D3D11 device".into()))?;
        let _ = device
            .cast::<ID3D11Multithread>()?
            .SetMultithreadProtected(true);
        let mut token = 0u32;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.ok_or_else(|| ExportError::Media("no device manager".into()))?;
        manager.ResetDevice(&device, token)?;
        Ok(manager)
    }
}

/// A DXGI device manager for an existing D3D11 device (video support, multithread-protected),
/// e.g. the one decode and render share for zero-copy export.
pub fn device_manager_for(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager> {
    startup()?;
    unsafe {
        let mut token = 0u32;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.ok_or_else(|| ExportError::Media("no device manager".into()))?;
        manager.ResetDevice(device, token)?;
        Ok(manager)
    }
}

/// The D3D11 device behind a manager (create input textures on this device).
pub fn manager_device(manager: &IMFDXGIDeviceManager) -> Result<ID3D11Device> {
    unsafe {
        let h = manager.OpenDeviceHandle()?;
        let mut raw = std::ptr::null_mut();
        let r = manager.GetVideoService(h, &ID3D11Device::IID, &mut raw);
        let _ = manager.CloseDeviceHandle(h);
        r?;
        Ok(ID3D11Device::from_raw(raw))
    }
}

#[derive(Default)]
pub struct ExportOptions {
    /// `None` = auto-pick (Intel, AMD, NVIDIA, then software).
    pub encoder: Option<EncoderInfo>,
    /// Zero-copy input: textures passed to [`Exporter::push_video_texture`] must live on
    /// this manager's device. `None` = the exporter creates one for a hardware encoder.
    pub device_manager: Option<IMFDXGIDeviceManager>,
    pub total_frames: Option<u64>,
    pub cancel: CancelToken,
    pub on_progress: Option<Box<dyn FnMut(Progress)>>,
}

#[derive(Debug, Clone)]
pub struct ExportStats {
    pub frames: u64,
    pub audio_frames: u64,
    /// Encoder MF actually loaded (from `IMFSinkWriterEx::GetTransformForStream`).
    pub encoder_name: String,
    pub encoder_vendor: Vendor,
}

/// Writes one MP4: video NV12 frames in, H.264/HEVC out; optional AAC-LC 48 kHz stereo.
/// Dropping it before [`Exporter::finish`] (error, cancel, panic) deletes the partial file.
/// Feed audio interleaved with video (about one frame's worth at a time); the sink
/// writer throttles a stream that runs far ahead.
pub struct Exporter {
    writer: Option<IMFSinkWriter>,
    path: PathBuf,
    preset: Preset,
    video: u32,
    audio: Option<u32>,
    frames: u64,
    audio_frames: u64,
    manager: Option<IMFDXGIDeviceManager>,
    allocator: Option<IMFVideoSampleAllocatorEx>,
    encoder_name: String,
    encoder_vendor: Vendor,
    total_frames: Option<u64>,
    cancel: CancelToken,
    on_progress: Option<Box<dyn FnMut(Progress)>>,
}

fn media_type() -> Result<IMFMediaType> {
    Ok(unsafe { MFCreateMediaType()? })
}

fn set_video_common(t: &IMFMediaType, p: &Preset) -> Result<()> {
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(p.width, p.height))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(p.fps_num, p.fps_den))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        // BT.709 primaries/transfer/matrix, limited range (D5).
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    }
    Ok(())
}

fn audio_type(subtype: &GUID, bytes_per_sec: u32) -> Result<IMFMediaType> {
    let t = media_type()?;
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_SAMPLE_RATE)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_sec)?;
    }
    Ok(t)
}

/// Approximate bitrate for the output type when the preset isn't bitrate-driven.
fn nominal_bps(p: &Preset) -> u32 {
    match p.video.rate_control {
        RateControl::Cbr { bitrate_kbps } | RateControl::Vbr { bitrate_kbps, .. } => {
            bitrate_kbps * 1000
        }
        _ => {
            let px_per_s = p.width as u64 * p.height as u64 * p.fps_num as u64 / p.fps_den as u64;
            (px_per_s / 2).min(u32::MAX as u64) as u32
        }
    }
}

/// Encoder settings handed to the MFT's ICodecAPI by the sink writer.
/// The Microsoft software MFT shifts every PTS by one frame when B-frames are on, and with
/// no edit list that breaks zero-based PTS (A-QUAL-07), so software encodes use none.
fn encoder_params(p: &Preset, hardware: bool) -> Result<IMFAttributes> {
    let mut a = None;
    unsafe {
        MFCreateAttributes(&mut a, 10)?;
        let a = a.ok_or_else(|| ExportError::Media("no attributes".into()))?;
        let mode = |m: eAVEncCommonRateControlMode| m.0 as u32;
        match p.video.rate_control {
            RateControl::Cbr { bitrate_kbps } => {
                a.SetUINT32(
                    &CODECAPI_AVEncCommonRateControlMode,
                    mode(eAVEncCommonRateControlMode_CBR),
                )?;
                a.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, bitrate_kbps * 1000)?;
            }
            RateControl::Vbr {
                bitrate_kbps,
                max_bitrate_kbps,
            } => {
                a.SetUINT32(
                    &CODECAPI_AVEncCommonRateControlMode,
                    mode(eAVEncCommonRateControlMode_PeakConstrainedVBR),
                )?;
                a.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, bitrate_kbps * 1000)?;
                a.SetUINT32(&CODECAPI_AVEncCommonMaxBitRate, max_bitrate_kbps * 1000)?;
            }
            RateControl::Quality { quality } => {
                a.SetUINT32(
                    &CODECAPI_AVEncCommonRateControlMode,
                    mode(eAVEncCommonRateControlMode_Quality),
                )?;
                a.SetUINT32(&CODECAPI_AVEncCommonQuality, quality)?;
            }
            RateControl::ConstQp { qp } => {
                a.SetUINT32(
                    &CODECAPI_AVEncCommonRateControlMode,
                    mode(eAVEncCommonRateControlMode_Quality),
                )?;
                a.SetUINT64(&CODECAPI_AVEncVideoEncodeQP, qp as u64)?;
                a.SetUINT32(&CODECAPI_AVEncVideoMinQP, qp)?;
                a.SetUINT32(&CODECAPI_AVEncVideoMaxQP, qp)?;
            }
        }
        a.SetUINT32(&CODECAPI_AVEncMPVGOPSize, p.gop_frames())?;
        a.SetUINT32(&CODECAPI_AVEncMPVGOPOpen, 0)?;
        let b_frames = if hardware { p.video.b_frames } else { 0 };
        a.SetUINT32(&CODECAPI_AVEncMPVDefaultBPictureCount, b_frames)?;
        if p.video.codec == VideoCodec::H264 {
            a.SetUINT32(&CODECAPI_AVEncH264CABACEnable, 1)?;
        }
        Ok(a)
    }
}

impl Exporter {
    pub fn new(path: &Path, preset: &Preset, opts: ExportOptions) -> Result<Self> {
        preset.validate()?;
        let encoder = match opts.encoder {
            Some(e) => e,
            None => enumerate_encoders(preset.video.codec)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    ExportError::Unsupported(format!(
                        "no {:?} encoder (HEVC needs the Store HEVC Video Extensions)",
                        preset.video.codec
                    ))
                })?,
        };
        if encoder.codec != preset.video.codec {
            return Err(ExportError::InvalidInput(
                "encoder codec differs from preset".into(),
            ));
        }
        startup()?;
        let manager = match opts.device_manager {
            Some(m) => Some(m),
            None if encoder.hardware => Some(create_device_manager(encoder.adapter_luid)?),
            None => None,
        };

        let mut exporter = Exporter {
            writer: None,
            path: path.to_path_buf(),
            preset: preset.clone(),
            video: 0,
            audio: None,
            frames: 0,
            audio_frames: 0,
            manager,
            allocator: None,
            encoder_name: String::new(),
            encoder_vendor: encoder.vendor,
            total_frames: opts.total_frames,
            cancel: opts.cancel,
            on_progress: opts.on_progress,
        };
        // From here on a failure drops `exporter`, which removes any partial file.
        exporter.open(&encoder)?;
        Ok(exporter)
    }

    fn open(&mut self, requested: &EncoderInfo) -> Result<()> {
        let hardware = requested.hardware;
        let p = &self.preset;
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = attrs.ok_or_else(|| ExportError::Media("no attributes".into()))?;
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32)?;
            attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            // MF_MPEG4SINK_MOOV_BEFORE_MDAT corrupts the file (URL sink: E_ACCESSDENIED on
            // Finalize; read/write byte stream: mdat header lost), so finish() runs
            // crate::mp4::faststart instead.
            if let Some(m) = &self.manager {
                attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, m)?;
            }
            let writer =
                MFCreateSinkWriterFromURL(&HSTRING::from(self.path.as_os_str()), None, &attrs)?;
            self.writer = Some(writer.clone());

            let out = media_type()?;
            set_video_common(&out, p)?;
            out.SetGUID(&MF_MT_SUBTYPE, &subtype(p.video.codec))?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, nominal_bps(p))?;
            let profile = match p.video.codec {
                VideoCodec::H264 => eAVEncH264VProfile_High.0 as u32,
                VideoCodec::Hevc => eAVEncH265VProfile_Main_420_8.0 as u32,
            };
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, profile)?;
            self.video = writer.AddStream(&out)?;

            let input = media_type()?;
            set_video_common(&input, p)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, p.width)?;
            writer.SetInputMediaType(self.video, &input, &encoder_params(p, hardware)?)?;

            if let Some(a) = &p.audio {
                let aac = audio_type(&MFAudioFormat_AAC, aac_bytes_per_second(a.bitrate_kbps))?;
                aac.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?; // AAC-LC L2
                let idx = writer.AddStream(&aac)?;
                let pcm = audio_type(&MFAudioFormat_PCM, AUDIO_SAMPLE_RATE * AUDIO_CHANNELS * 2)?;
                pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, AUDIO_CHANNELS * 2)?;
                writer.SetInputMediaType(idx, &pcm, None)?;
                self.audio = Some(idx);
            }
            writer.BeginWriting()?;
            let (name, vendor) = actual_encoder(&writer, self.video);
            self.encoder_vendor = vendor.unwrap_or(requested.vendor);
            // Loaded MFTs often carry no friendly name; the requested one is the best label.
            self.encoder_name = name.unwrap_or_else(|| requested.name.clone());
        }
        Ok(())
    }

    /// Name and vendor of the encoder MF loaded (may differ from the request).
    pub fn encoder(&self) -> (&str, Vendor) {
        (&self.encoder_name, self.encoder_vendor)
    }

    /// Device manager in use, if any (create zero-copy textures on its device).
    pub fn device_manager(&self) -> Option<&IMFDXGIDeviceManager> {
        self.manager.as_ref()
    }

    fn writer(&self) -> Result<&IMFSinkWriter> {
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        self.writer
            .as_ref()
            .ok_or_else(|| ExportError::Media("writer closed".into()))
    }

    fn write_video(&mut self, buffer: IMFMediaBuffer) -> Result<()> {
        let s = unsafe { MFCreateSample()? };
        unsafe { s.AddBuffer(&buffer)? };
        self.write_sample(s)
    }

    fn write_sample(&mut self, s: IMFSample) -> Result<()> {
        let writer = self.writer()?;
        let t0 = self.preset.frame_time_hns(self.frames);
        let t1 = self.preset.frame_time_hns(self.frames + 1);
        unsafe {
            s.SetSampleTime(t0)?;
            s.SetSampleDuration(t1 - t0)?;
            writer.WriteSample(self.video, &s)?;
        }
        self.frames += 1;
        let progress = Progress {
            frames_written: self.frames,
            total_frames: self.total_frames,
        };
        if let Some(cb) = &mut self.on_progress {
            cb(progress);
        }
        Ok(())
    }

    /// Tightly packed NV12 (Y plane then interleaved UV, stride = width), BT.709 limited.
    pub fn push_video_nv12(&mut self, nv12: &[u8]) -> Result<()> {
        let len = self.preset.width as usize * self.preset.height as usize * 3 / 2;
        if nv12.len() != len {
            return Err(ExportError::InvalidInput(format!(
                "nv12 frame is {} bytes, expected {len}",
                nv12.len()
            )));
        }
        self.writer()?;
        let buffer = unsafe {
            let b = MFCreateMemoryBuffer(len as u32)?;
            let mut ptr = std::ptr::null_mut();
            b.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), ptr, len);
            b.Unlock()?;
            b.SetCurrentLength(len as u32)?;
            b
        };
        self.write_video(buffer)
    }

    /// RGBA8 (full range), converted on the CPU to NV12 BT.709 limited.
    pub fn push_video_rgba(&mut self, rgba: &[u8]) -> Result<()> {
        let nv12 = rgba_to_nv12(
            rgba,
            self.preset.width as usize,
            self.preset.height as usize,
        )?;
        self.push_video_nv12(&nv12)
    }

    /// Zero-copy: an NV12 `ID3D11Texture2D` on the device manager's device. MF may hold
    /// the texture until it is encoded, so don't overwrite it; use a fresh texture or pool.
    pub fn push_video_texture(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
    ) -> Result<()> {
        if self.manager.is_none() {
            return Err(ExportError::InvalidInput(
                "texture input needs a device manager".into(),
            ));
        }
        self.writer()?;
        let buffer = unsafe {
            let b = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, subresource, false)?;
            let len = b.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
            b.SetCurrentLength(len)?;
            b
        };
        self.write_video(buffer)
    }

    /// Zero-copy with reuse: `copy` fills a pooled NV12 texture (on the device manager's
    /// device, e.g. a GPU-side `CopySubresourceRegion` of the caller's frame), which is then
    /// written. MF returns the texture to the pool once encoded, so the caller's own texture is
    /// free as soon as its copy has run on the GPU.
    pub fn push_video_copy(
        &mut self,
        copy: impl FnOnce(&ID3D11Texture2D, u32) -> Result<()>,
    ) -> Result<()> {
        self.writer()?;
        let alloc = match &self.allocator {
            Some(a) => a.clone(),
            None => {
                let m = self.manager.as_ref().ok_or_else(|| {
                    ExportError::InvalidInput("texture input needs a device manager".into())
                })?;
                let a = unsafe {
                    let mut p = std::ptr::null_mut();
                    MFCreateVideoSampleAllocatorEx(&IMFVideoSampleAllocatorEx::IID, &mut p)?;
                    IMFVideoSampleAllocatorEx::from_raw(p)
                };
                let t = media_type()?;
                set_video_common(&t, &self.preset)?;
                unsafe {
                    t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                    a.SetDirectXManager(m)?;
                    let mut attrs = None;
                    MFCreateAttributes(&mut attrs, 1)?;
                    a.InitializeSampleAllocatorEx(2, 16, attrs.as_ref(), &t)?;
                }
                self.allocator = Some(a.clone());
                a
            }
        };
        let sample = loop {
            match unsafe { alloc.AllocateSample() } {
                Ok(s) => break s,
                // All pooled samples are still queued in the encoder: wait for one.
                Err(e) if e.code() == MF_E_SAMPLEALLOCATOR_EMPTY => {
                    self.writer()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(e) => return Err(e.into()),
            }
        };
        let (texture, sub) = unsafe {
            let b = sample.GetBufferByIndex(0)?;
            b.SetCurrentLength(b.cast::<IMF2DBuffer>()?.GetContiguousLength()?)?;
            let dxgi: IMFDXGIBuffer = b.cast()?;
            let mut p = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut p)?;
            (ID3D11Texture2D::from_raw(p), dxgi.GetSubresourceIndex()?)
        };
        copy(&texture, sub)?;
        self.write_sample(sample)
    }

    /// f32 interleaved stereo at 48 kHz. Samples are clamped to [-1, 1].
    pub fn push_audio(&mut self, samples: &[f32]) -> Result<()> {
        let idx = self
            .audio
            .ok_or_else(|| ExportError::InvalidInput("preset has no audio".into()))?;
        if samples.is_empty() || !samples.len().is_multiple_of(AUDIO_CHANNELS as usize) {
            return Err(ExportError::InvalidInput(
                "audio must be non-empty interleaved stereo".into(),
            ));
        }
        let writer = self.writer()?;
        let frames = (samples.len() / AUDIO_CHANNELS as usize) as u64;
        let at = |f: u64| (f as u128 * 10_000_000 / AUDIO_SAMPLE_RATE as u128) as i64;
        let t0 = at(self.audio_frames);
        let t1 = at(self.audio_frames + frames);
        unsafe {
            let bytes = samples.len() * 2;
            let b = MFCreateMemoryBuffer(bytes as u32)?;
            let mut ptr = std::ptr::null_mut();
            b.Lock(&mut ptr, None, None)?;
            let dst = std::slice::from_raw_parts_mut(ptr as *mut i16, samples.len());
            for (d, s) in dst.iter_mut().zip(samples) {
                *d = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            }
            b.Unlock()?;
            b.SetCurrentLength(bytes as u32)?;
            let s = MFCreateSample()?;
            s.AddBuffer(&b)?;
            s.SetSampleTime(t0)?;
            s.SetSampleDuration(t1 - t0)?;
            writer.WriteSample(idx, &s)?;
        }
        self.audio_frames += frames;
        Ok(())
    }

    /// Finalize the MP4. On error the partial file is removed.
    pub fn finish(mut self) -> Result<ExportStats> {
        let writer = self.writer()?.clone();
        unsafe { writer.Finalize()? };
        drop(writer);
        self.writer = None;
        if let Err(e) = crate::mp4::faststart(&self.path) {
            let _ = std::fs::remove_file(&self.path);
            return Err(e);
        }
        Ok(ExportStats {
            frames: self.frames,
            audio_frames: self.audio_frames,
            encoder_name: std::mem::take(&mut self.encoder_name),
            encoder_vendor: self.encoder_vendor,
        })
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        // `writer` is still set only if finish() didn't succeed: shut the sink down so it
        // releases the file, then delete the partial output.
        if let Some(w) = self.writer.take() {
            unsafe {
                let mut raw = std::ptr::null_mut();
                if w.GetServiceForStream(
                    MF_SINK_WRITER_MEDIASINK.0,
                    &GUID::zeroed(),
                    &IMFMediaSink::IID,
                    &mut raw,
                )
                .is_ok()
                {
                    let _ = IMFMediaSink::from_raw(raw).Shutdown();
                }
            }
            drop(w);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Name and vendor of the video encoder MFT the sink writer actually loaded, where known.
fn actual_encoder(writer: &IMFSinkWriter, stream: u32) -> (Option<String>, Option<Vendor>) {
    let Ok(ex) = writer.cast::<IMFSinkWriterEx>() else {
        return (None, None);
    };
    for i in 0.. {
        let mut cat = GUID::zeroed();
        let mut tf = None;
        if unsafe { ex.GetTransformForStream(stream, i, Some(&mut cat), &mut tf) }.is_err() {
            break;
        }
        if cat != MFT_CATEGORY_VIDEO_ENCODER {
            continue;
        }
        let Some(attrs) = tf.and_then(|t| unsafe { t.GetAttributes() }.ok()) else {
            break;
        };
        let name = get_string(&attrs, &MFT_FRIENDLY_NAME_Attribute);
        let vendor_id = get_string(&attrs, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute);
        let hardware =
            vendor_id.is_some() || get_string(&attrs, &MFT_ENUM_HARDWARE_URL_Attribute).is_some();
        let vendor = Vendor::identify(
            vendor_id.as_deref(),
            name.as_deref().unwrap_or(""),
            hardware,
        );
        return (name, Some(vendor));
    }
    (None, None)
}
