//! Media Foundation: frame-exact video decode (D3D11/DXVA or CPU), audio decode, H.264 proxies.

use super::{GpuFrame, err, init};
use crate::{
    AudioBuffer, AudioStream, Error, Frame, FrameData, HNS, Hdr, Index, PixelFormat, ProbeInfo,
    Rational, Result, Tools, VideoStream,
};
use std::path::{Path, PathBuf};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VT_I8, VT_UI4};
use windows_core::{GUID, HSTRING, Interface};

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
const HEVC_HELP: &str = "This HEVC (H.265) video needs the free or $0.99 \"HEVC Video Extensions\" \
from the Microsoft Store: ms-windows-store://pdp/?ProductId=9NMZLZ57R3T7";

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn even(x: u32) -> u32 {
    (x & !1).max(2)
}

/// Source reader for `path`, optionally on `device` (DXVA + hardware MFTs), with one stream type
/// (`VIDEO` or `AUDIO`) selected.
fn open_reader(
    path: &Path,
    device: Option<&ID3D11Device>,
    stream: u32,
) -> Result<(IMFSourceReader, Option<IMFDXGIDeviceManager>)> {
    init()?;
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 4).map_err(err("MFCreateAttributes"))?;
        let attrs = attrs.ok_or_else(|| Error::Codec("no MF attributes".into()))?;
        let mut manager = None;
        if let Some(dev) = device {
            super::protect(dev)?;
            let mut token = 0;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)
                .map_err(err("DXGI device manager"))?;
            let m = manager
                .as_ref()
                .ok_or_else(|| Error::Codec("no device manager".into()))?;
            m.ResetDevice(dev, token).map_err(err("ResetDevice"))?;
            attrs
                .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, m)
                .map_err(err("attrs"))?;
            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(err("attrs"))?;
        }
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .map_err(err("attrs"))?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path), &attrs)
            .map_err(|e| Error::Unsupported(format!("Windows can't open this file: {e}")))?;
        reader
            .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
            .map_err(err("stream selection"))?;
        reader.SetStreamSelection(stream, true).map_err(|_| {
            Error::Unsupported("Windows found no playable stream of that kind".into())
        })?;
        Ok((reader, manager))
    }
}

fn media_type(major: &GUID, sub: &GUID) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType().map_err(err("MFCreateMediaType"))?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, major)
            .map_err(err("media type"))?;
        t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(err("media type"))?;
        Ok(t)
    }
}

fn set_video_output(
    reader: &IMFSourceReader,
    ten_bit: bool,
    size: Option<(u32, u32)>,
) -> Result<PixelFormat> {
    let tries: &[(GUID, PixelFormat)] = if ten_bit {
        &[
            (MFVideoFormat_P010, PixelFormat::P010),
            (MFVideoFormat_NV12, PixelFormat::Nv12),
        ]
    } else {
        &[(MFVideoFormat_NV12, PixelFormat::Nv12)]
    };
    let mut last = None;
    for (sub, fmt) in tries {
        let t = media_type(&MFMediaType_Video, sub)?;
        unsafe {
            if let Some((w, h)) = size {
                t.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))
                    .map_err(err("frame size"))?;
            }
            match reader.SetCurrentMediaType(VIDEO, None, &t) {
                Ok(()) => return Ok(*fmt),
                Err(e) => last = Some(e),
            }
        }
    }
    Err(Error::Unsupported(format!(
        "Windows has no decoder for this video: {}",
        last.map(|e| e.message()).unwrap_or_default()
    )))
}

fn seek(reader: &IMFSourceReader, hns: i64) -> Result<()> {
    let pv = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I8,
                Anonymous: windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0_0 {
                    hVal: hns,
                },
                ..Default::default()
            }),
        },
    };
    unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &pv) }.map_err(err("seek"))
}

/// One read: `Ok(None)` at end of stream.
fn read(reader: &IMFSourceReader, stream: u32) -> Result<Option<(i64, IMFSample)>> {
    loop {
        let (mut flags, mut ts, mut sample) = (0u32, 0i64, None);
        unsafe {
            reader
                .ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut ts),
                    Some(&mut sample),
                )
                .map_err(err("ReadSample"))?;
        }
        let flags = MF_SOURCE_READER_FLAG(flags as i32);
        if flags.contains(MF_SOURCE_READERF_ERROR) {
            return Err(Error::Codec("decoder error".into()));
        }
        if let Some(s) = sample {
            return Ok(Some((ts, s)));
        }
        if flags.contains(MF_SOURCE_READERF_ENDOFSTREAM) {
            return Ok(None);
        }
    }
}

/// Fills bit depth (`pix_fmt`) and colour of the first H.264/HEVC video stream from Media
/// Foundation's native media type (container + codec config: no decode, no HEVC extension) where
/// ffprobe left them unknown: the shipped LGPL ffprobe has no H.264/HEVC decoder or parser (D3,
/// D23). Best effort: whatever MF can't tell stays `None`.
pub(crate) fn fill_native(info: &mut ProbeInfo) {
    let Some(v) = info.video.first_mut() else {
        return;
    };
    let hevc = match v.codec.as_str() {
        "hevc" => true,
        "h264" => false,
        _ => return,
    };
    let c = &v.color;
    if v.pix_fmt.is_some()
        && c.primaries.is_some()
        && c.transfer.is_some()
        && c.matrix.is_some()
        && c.range.is_some()
    {
        return;
    }
    let Ok(t) = open_reader(&info.path, None, VIDEO)
        .and_then(|(r, _)| unsafe { r.GetNativeMediaType(VIDEO, 0) }.map_err(err("native type")))
    else {
        return;
    };
    let get = |k: &GUID| unsafe { t.GetUINT32(k) }.ok();
    let n = native_names(
        hevc,
        get(&MF_MT_VIDEO_PROFILE),
        [
            get(&MF_MT_VIDEO_PRIMARIES),
            get(&MF_MT_TRANSFER_FUNCTION),
            get(&MF_MT_YUV_MATRIX),
            get(&MF_MT_VIDEO_NOMINAL_RANGE),
        ],
    );
    let fill = |f: &mut Option<String>, s: Option<&str>| {
        if f.is_none() {
            *f = s.map(str::to_owned);
        }
    };
    fill(&mut v.pix_fmt, n[0]);
    let c = &mut v.color;
    fill(&mut c.primaries, n[1]);
    fill(&mut c.transfer, n[2]);
    fill(&mut c.matrix, n[3]);
    fill(&mut c.range, n[4]);
    c.hdr = c.hdr.or(match c.transfer.as_deref() {
        Some("arib-std-b67") => Some(Hdr::Hlg),
        Some("smpte2084") => Some(Hdr::Pq),
        _ => None,
    });
}

/// MF values (`MF_MT_VIDEO_PROFILE`, then `[primaries, transfer, matrix, range]`) -> FFmpeg names
/// `[pix_fmt, primaries, transfer, matrix, range]`. Unknown or ambiguous values (DCI-P3 primaries,
/// 4:2:2/4:4:4 or 12-bit profiles) map to `None`.
fn native_names(
    hevc: bool,
    profile: Option<u32>,
    [pri, trc, mat, rng]: [Option<u32>; 4],
) -> [Option<&'static str>; 5] {
    let i = |x: Option<u32>| x.map(|x| x as i32).unwrap_or(-1);
    let pix = match (hevc, i(profile)) {
        (true, 1 | 11 | 20) => Some("yuv420p"), // Main, MainIntra 8, MainStill
        (true, 2 | 12) => Some("yuv420p10le"),  // Main 10, MainIntra 10
        (false, 66 | 77 | 88 | 100 | 256 | 257) => Some("yuv420p"),
        (false, 110) => Some("yuv420p10le"), // High 10
        _ => None,
    };
    let primaries = match i(pri) {
        x if x == MFVideoPrimaries_BT709.0 => Some("bt709"),
        x if x == MFVideoPrimaries_BT470_2_SysM.0 => Some("bt470m"),
        x if x == MFVideoPrimaries_BT470_2_SysBG.0 => Some("bt470bg"),
        x if x == MFVideoPrimaries_SMPTE170M.0 => Some("smpte170m"),
        x if x == MFVideoPrimaries_SMPTE240M.0 => Some("smpte240m"),
        x if x == MFVideoPrimaries_BT2020.0 => Some("bt2020"),
        _ => None,
    };
    let transfer = match i(trc) {
        x if x == MFVideoTransFunc_709.0 => Some("bt709"),
        x if x == MFVideoTransFunc_240M.0 => Some("smpte240m"),
        x if x == MFVideoTransFunc_sRGB.0 => Some("iec61966-2-1"),
        x if x == MFVideoTransFunc_22.0 => Some("gamma22"),
        x if x == MFVideoTransFunc_28.0 => Some("gamma28"),
        x if x == MFVideoTransFunc_10.0 => Some("linear"),
        x if x == MFVideoTransFunc_2020.0 => Some("bt2020-10"),
        x if x == MFVideoTransFunc_2084.0 => Some("smpte2084"),
        x if x == MFVideoTransFunc_HLG.0 => Some("arib-std-b67"),
        _ => None,
    };
    let matrix = match i(mat) {
        x if x == MFVideoTransferMatrix_BT709.0 => Some("bt709"),
        x if x == MFVideoTransferMatrix_BT601.0 => Some("smpte170m"),
        x if x == MFVideoTransferMatrix_SMPTE240M.0 => Some("smpte240m"),
        x if x == MFVideoTransferMatrix_BT2020_10.0 || x == MFVideoTransferMatrix_BT2020_12.0 => {
            Some("bt2020nc")
        }
        _ => None,
    };
    let range = match i(rng) {
        x if x == MFNominalRange_16_235.0 => Some("tv"),
        x if x == MFNominalRange_0_255.0 => Some("pc"),
        _ => None,
    };
    [pix, primaries, transfer, matrix, range]
}

/// Frame-exact video decoder for one file. Not `Send`: keep it on the thread that opened it.
pub struct VideoDecoder {
    reader: IMFSourceReader,
    _manager: Option<IMFDXGIDeviceManager>,
    device: Option<ID3D11Device>,
    staging: Option<ID3D11Texture2D>,
    index: Index,
    width: u32,
    height: u32,
    alloc: (u32, u32),
    format: PixelFormat,
    /// Half the smallest frame interval, in 100 ns: how far an MF timestamp may sit from ours.
    tol: i64,
    /// MF sample time minus our PTS (100 ns). MF ignores some edit lists (e.g. the B-frame
    /// composition shift), so we measure it from the first frame instead of trusting either.
    offset: i64,
    /// Last sample read from the reader (decoder position), time already offset-corrected.
    last: Option<(i64, IMFSample)>,
    readback: bool,
}

impl VideoDecoder {
    /// Opens the first video stream. With `device`, decode runs on that D3D11 device (DXVA) and
    /// frames are GPU textures unless [`set_cpu_readback`](Self::set_cpu_readback) is on.
    pub fn open(
        path: &Path,
        v: &VideoStream,
        index: Index,
        device: Option<&ID3D11Device>,
    ) -> Result<Self> {
        let hevc = v.codec == "hevc";
        let opened = open_reader(path, device, VIDEO).and_then(|(r, m)| {
            let ten = v.pix_fmt.as_deref().is_some_and(|p| p.contains("10"));
            let fmt = set_video_output(&r, ten, None)?;
            Ok((r, m, fmt))
        });
        let (reader, manager, format) = opened.map_err(|e| match e {
            Error::Unsupported(_) if hevc => Error::Unsupported(HEVC_HELP.into()),
            e => e,
        })?;
        let cur = unsafe { reader.GetCurrentMediaType(VIDEO) }.map_err(err("current type"))?;
        let alloc = unpack(unsafe { cur.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(err("frame size"))?);
        let tb = index.time_base;
        let (t0, s0) =
            read(&reader, VIDEO)?.ok_or_else(|| Error::Codec("the video has no frames".into()))?;
        let offset = t0 - tb.rescale(index.pts[0], HNS);
        let min_delta = index
            .pts
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|&d| d > 0)
            .min()
            .unwrap_or(1);
        Ok(Self {
            reader,
            _manager: manager,
            device: device.cloned(),
            staging: None,
            tol: (tb.rescale(min_delta, HNS) / 2 - 1).max(1),
            offset,
            last: Some((t0 - offset, s0)),
            index,
            width: v.width.min(alloc.0),
            height: v.height.min(alloc.1),
            alloc,
            format,
            readback: device.is_none(),
        })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Return CPU (NV12/P010) frames even when decoding on the GPU (tests, thumbnails).
    pub fn set_cpu_readback(&mut self, on: bool) {
        self.readback = on || self.device.is_none();
    }

    fn hns(&self, pts: i64) -> i64 {
        self.index.time_base.rescale(pts, HNS)
    }

    /// The frame shown at `pts` (stream time base): the latest frame with PTS <= `pts`.
    /// Seeks to the keyframe at or before it and decodes forward, or just decodes forward when the
    /// target is ahead in the same GOP.
    pub fn frame_at(&mut self, pts: i64) -> Result<Frame> {
        let fpts = self.index.pts[self.index.frame_at(pts)];
        let target = self.hns(fpts);
        let tol = self.tol;
        let near = |t: i64| (t - target).abs() <= tol;
        if let Some((t, s)) = &self.last
            && near(*t)
        {
            let s = s.clone();
            return self.make_frame(fpts, s);
        }
        let tb = self.index.time_base;
        let must_seek = match &self.last {
            None => true,
            Some((t, _)) => *t > target || self.index.keyframe_between(HNS.rescale(*t, tb), fpts),
        };
        if must_seek {
            let kf = self.index.keyframe_before(fpts);
            // MF seeks to the keyframe at or before the position; aim just past ours for rounding.
            seek(&self.reader, (self.hns(kf) + self.offset + 1).max(0))?;
            self.last = None;
        }
        let mut prev: Option<IMFSample> = None;
        loop {
            match read(&self.reader, VIDEO)? {
                None => {
                    // Past the last frame: hold the last one we saw.
                    let s = prev
                        .or_else(|| self.last.take().map(|l| l.1))
                        .ok_or_else(|| Error::Codec("no frame at that time".into()))?;
                    self.last = None;
                    return self.make_frame(fpts, s);
                }
                Some((t, s)) => {
                    let t = t - self.offset;
                    self.last = Some((t, s.clone()));
                    if near(t) {
                        return self.make_frame(fpts, s);
                    }
                    if t > target {
                        // The frame we wanted isn't in the stream: sample-and-hold.
                        return self.make_frame(fpts, prev.unwrap_or(s));
                    }
                    prev = Some(s);
                }
            }
        }
    }

    fn make_frame(&mut self, pts: i64, sample: IMFSample) -> Result<Frame> {
        let buffer = unsafe { sample.GetBufferByIndex(0) }.map_err(err("sample buffer"))?;
        if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
            let texture: ID3D11Texture2D = unsafe {
                let mut p = std::ptr::null_mut();
                dxgi.GetResource(&ID3D11Texture2D::IID, &mut p)
                    .map_err(err("GetResource"))?;
                ID3D11Texture2D::from_raw(p)
            };
            let subresource = unsafe { dxgi.GetSubresourceIndex() }.map_err(err("subresource"))?;
            let frame = Frame {
                pts,
                width: self.width,
                height: self.height,
                format: self.format,
                data: FrameData::D3D11(GpuFrame {
                    texture,
                    subresource,
                    _sample: sample,
                }),
            };
            return if self.readback {
                self.to_cpu(&frame)
            } else {
                Ok(frame)
            };
        }
        self.cpu_frame(pts, &buffer)
    }

    fn bytes_per_sample(&self) -> usize {
        if self.format == PixelFormat::P010 {
            2
        } else {
            1
        }
    }

    /// Copies the visible Y rows and UV rows out of a plane layout with `pitch` and `rows` luma
    /// rows allocated.
    fn pack_planes(&self, base: *const u8, pitch: usize, rows: usize) -> Vec<u8> {
        let (h, row) = (
            self.height as usize,
            self.width as usize * self.bytes_per_sample(),
        );
        let mut out = vec![0u8; pitch * (h + h / 2)];
        for y in 0..h {
            let src = unsafe { std::slice::from_raw_parts(base.add(y * pitch), row) };
            out[y * pitch..y * pitch + row].copy_from_slice(src);
        }
        for y in 0..h / 2 {
            let src = unsafe { std::slice::from_raw_parts(base.add((rows + y) * pitch), row) };
            let o = (h + y) * pitch;
            out[o..o + row].copy_from_slice(src);
        }
        out
    }

    fn cpu_frame(&self, pts: i64, buffer: &IMFMediaBuffer) -> Result<Frame> {
        let rows = self.alloc.1 as usize;
        let bytes = unsafe {
            if let Ok(b2) = buffer.cast::<IMF2DBuffer>() {
                let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
                b2.Lock2D(&mut p, &mut pitch).map_err(err("Lock2D"))?;
                let pitch = pitch.unsigned_abs() as usize;
                let out = self.pack_planes(p, pitch, rows);
                let _ = b2.Unlock2D();
                (out, pitch)
            } else {
                let mut p = std::ptr::null_mut();
                buffer.Lock(&mut p, None, None).map_err(err("Lock"))?;
                let pitch = self.alloc.0 as usize * self.bytes_per_sample();
                let out = self.pack_planes(p, pitch, rows);
                let _ = buffer.Unlock();
                (out, pitch)
            }
        };
        Ok(Frame {
            pts,
            width: self.width,
            height: self.height,
            format: self.format,
            data: FrameData::Cpu {
                bytes: bytes.0,
                stride: bytes.1,
            },
        })
    }

    /// GPU frame -> CPU NV12/P010 via a staging texture (CPU frames are cloned).
    pub fn to_cpu(&mut self, frame: &Frame) -> Result<Frame> {
        let g = match &frame.data {
            FrameData::Cpu { bytes, stride } => {
                return Ok(Frame {
                    data: FrameData::Cpu {
                        bytes: bytes.clone(),
                        stride: *stride,
                    },
                    ..*frame
                });
            }
            FrameData::D3D11(g) => g,
        };
        let device = self
            .device
            .clone()
            .ok_or_else(|| Error::Codec("no device".into()))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { g.texture.GetDesc(&mut desc) };
        let staging = match &self.staging {
            Some(s) => s.clone(),
            None => {
                let sd = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut t = None;
                unsafe { device.CreateTexture2D(&sd, None, Some(&mut t)) }
                    .map_err(err("staging texture"))?;
                let t = t.ok_or_else(|| Error::Codec("no staging texture".into()))?;
                self.staging = Some(t.clone());
                t
            }
        };
        let mt: ID3D11Multithread = device.cast().map_err(err("multithread"))?;
        let ctx = unsafe { device.GetImmediateContext() }.map_err(err("context"))?;
        let bytes = unsafe {
            mt.Enter();
            ctx.CopySubresourceRegion(&staging, 0, 0, 0, 0, &g.texture, g.subresource, None);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            let r = ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m));
            let out = r.map(|()| {
                let pitch = m.RowPitch as usize;
                let b = self.pack_planes(m.pData as *const u8, pitch, desc.Height as usize);
                ctx.Unmap(&staging, 0);
                (b, pitch)
            });
            mt.Leave();
            out.map_err(err("Map"))?
        };
        Ok(Frame {
            pts: frame.pts,
            width: frame.width,
            height: frame.height,
            format: frame.format,
            data: FrameData::Cpu {
                bytes: bytes.0,
                stride: bytes.1,
            },
        })
    }
}

/// MF source reader for the first audio stream as float PCM: (reader, rate, channels).
fn mf_audio(path: &Path) -> Result<(IMFSourceReader, u32, u32)> {
    let (reader, _) = open_reader(path, None, AUDIO)?;
    let t = media_type(&MFMediaType_Audio, &MFAudioFormat_Float)?;
    unsafe {
        reader.SetCurrentMediaType(AUDIO, None, &t).map_err(|e| {
            Error::Unsupported(format!(
                "Windows has no decoder for this audio: {}",
                e.message()
            ))
        })?;
        let cur = reader
            .GetCurrentMediaType(AUDIO)
            .map_err(err("audio type"))?;
        let rate = cur
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .map_err(err("rate"))?;
        let channels = cur
            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
            .map_err(err("channels"))?;
        Ok((reader, rate, channels))
    }
}

/// Streams the audio as f32 48 kHz interleaved chunks to `sink(channels, samples)`, bounded
/// memory. Media Foundation first; FFmpeg only for royalty-free codecs MF can't open (never AAC).
/// Sample 0 is presentation time 0 of the file: a late first sample is padded with silence and
/// anything timed before 0 (encoder priming) is dropped.
pub fn stream_audio(
    tools: &Tools,
    path: &Path,
    a: &AudioStream,
    sink: &mut dyn FnMut(u32, &[f32]) -> Result<()>,
) -> Result<()> {
    let (reader, rate, ch) = match mf_audio(path) {
        Ok(r) => r,
        Err(e) => return crate::ffmpeg::stream_audio(tools, path, a, sink).map_err(|_| e),
    };
    let mut r = crate::To48k::new(rate, ch)?;
    // Output frames still to drop (negative start) before passing samples on.
    let drop = std::cell::Cell::new(0usize);
    let mut out = |s: &[f32]| {
        let n = (drop.get() * ch as usize).min(s.len());
        drop.set(drop.get() - n / ch as usize);
        if n < s.len() {
            sink(ch, &s[n..])
        } else {
            Ok(())
        }
    };
    let mut first = true;
    while let Some((ts, s)) = read(&reader, AUDIO)? {
        if std::mem::take(&mut first) {
            // MF ignores MP4 edit lists and keeps encoder priming: its first sample is the first
            // packet, whose presentation time ffprobe reports as the stream start (start delay
            // and priming included). Fall back to MF's own timestamp without a probe value.
            let at48 = Rational::new(1, crate::SAMPLE_RATE as i64);
            let lead = match a.start_us {
                Some(us) => crate::MICROS.rescale(us, at48),
                None => HNS.rescale(ts, at48),
            };
            if lead > 0 {
                out(&vec![0.0; lead as usize * ch as usize])?;
            } else {
                drop.set(lead.unsigned_abs() as usize);
            }
        }
        unsafe {
            let b = s.ConvertToContiguousBuffer().map_err(err("audio buffer"))?;
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            b.Lock(&mut p, None, Some(&mut len))
                .map_err(err("audio lock"))?;
            let f = std::slice::from_raw_parts(p as *const f32, len as usize / 4).to_vec();
            let _ = b.Unlock();
            r.push(&f, &mut out)?;
        }
    }
    r.finish(&mut out)
}

/// Decodes the whole stream into memory (tests, short clips); see [`stream_audio`].
pub fn decode_audio(tools: &Tools, path: &Path, a: &AudioStream) -> Result<AudioBuffer> {
    let mut buf = AudioBuffer {
        sample_rate: crate::SAMPLE_RATE,
        channels: a.channels.max(1),
        samples: Vec::new(),
    };
    stream_audio(tools, path, a, &mut |ch, s| {
        buf.channels = ch;
        buf.samples.extend_from_slice(s);
        Ok(())
    })?;
    Ok(buf)
}

/// Where the proxy for `path` lives (whether or not it exists yet).
pub fn proxy_path(path: &Path, cache_dir: &Path) -> Result<PathBuf> {
    Ok(cache_dir.join(format!("{}.proxy.mp4", crate::cache_key(path)?)))
}

fn variant_u32(v: u32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_UI4,
                Anonymous: windows::Win32::System::Variant::VARIANT_0_0_0 { ulVal: v },
                ..Default::default()
            }),
        },
    }
}

/// Encodes a short-GOP (<= 1 s, no B-frames) H.264 proxy, at most 1080 lines, with the MF H.264
/// encoder (hardware preferred). Video only: editing uses the original audio. Returns the cached
/// proxy at once if it exists. `progress(0.0..=1.0)` returning false cancels.
pub fn make_proxy(
    path: &Path,
    v: &VideoStream,
    duration_us: i64,
    cache_dir: &Path,
    device: Option<&ID3D11Device>,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<PathBuf> {
    let out = proxy_path(path, cache_dir)?;
    if out.is_file() {
        return Ok(out);
    }
    std::fs::create_dir_all(cache_dir)?;
    let part = out.with_extension("part.mp4");
    let res = encode_proxy(path, v, duration_us, &part, device, progress);
    match res {
        Ok(()) => {
            std::fs::rename(&part, &out)?;
            Ok(out)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn encode_proxy(
    path: &Path,
    v: &VideoStream,
    duration_us: i64,
    part: &Path,
    device: Option<&ID3D11Device>,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<()> {
    let (reader, manager) = open_reader(path, device, VIDEO)?;
    let (sw, sh) = (v.width.max(2), v.height.max(2));
    let h = even(sh.min(1080));
    let w = even((sw as u64 * h as u64 / sh as u64) as u32);
    set_video_output(&reader, false, Some((w, h)))?;
    let fps = v.avg_fps.or(v.r_fps).unwrap_or(Rational::new(30, 1));
    let gop = fps.as_f64().round().max(1.0) as u32;
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 4).map_err(err("MFCreateAttributes"))?;
        let attrs = attrs.ok_or_else(|| Error::Codec("no MF attributes".into()))?;
        attrs
            .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
            .map_err(err("attrs"))?;
        attrs
            .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
            .map_err(err("attrs"))?;
        if let Some(m) = &manager {
            attrs
                .SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, m)
                .map_err(err("attrs"))?;
        }
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(part), None, &attrs)
            .map_err(err("sink writer"))?;
        let set_common = |t: &IMFMediaType| -> Result<()> {
            t.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))
                .map_err(err("type"))?;
            t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps.num as u32, fps.den as u32))
                .map_err(err("type"))?;
            t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
                .map_err(err("type"))?;
            t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(err("type"))?;
            Ok(())
        };
        let out_t = media_type(&MFMediaType_Video, &MFVideoFormat_H264)?;
        set_common(&out_t)?;
        let bitrate = (w as u64 * h as u64 * 6).min(16_000_000) as u32; // ~12 Mbit/s at 1080p
        out_t
            .SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
            .map_err(err("type"))?;
        let stream = writer.AddStream(&out_t).map_err(err("AddStream"))?;
        let in_t = media_type(&MFMediaType_Video, &MFVideoFormat_NV12)?;
        set_common(&in_t)?;
        writer
            .SetInputMediaType(stream, &in_t, None)
            .map_err(|e| Error::Codec(format!("no H.264 encoder for {w}x{h}: {}", e.message())))?;
        let mut p = std::ptr::null_mut();
        if writer
            .GetServiceForStream(stream, &GUID::zeroed(), &ICodecAPI::IID, &mut p)
            .is_ok()
        {
            let api = ICodecAPI::from_raw(p);
            let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));
            let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
        }
        writer.BeginWriting().map_err(err("BeginWriting"))?;
        let total = (duration_us.max(1) * 10) as f32;
        let mut first = None;
        while let Some((ts, sample)) = read(&reader, VIDEO)? {
            let t0 = *first.get_or_insert(ts);
            sample.SetSampleTime(ts - t0).map_err(err("sample time"))?;
            writer
                .WriteSample(stream, &sample)
                .map_err(err("WriteSample"))?;
            if !progress(((ts - t0) as f32 / total).clamp(0.0, 1.0)) {
                return Err(Error::Cancelled);
            }
        }
        writer.Finalize().map_err(err("Finalize"))?;
    }
    progress(1.0);
    Ok(())
}
