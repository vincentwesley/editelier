//! Media the open project uses: probe results, frame indexes, thumbnails, proxies and decoded
//! audio, plus the per-thread decoder pool that turns (media, source time) into a frame.

use crate::api::{ApiError, ApiResult};
use editelier_core::{FrameRate, Id, MediaKind, MediaMeta, TICKS_PER_SECOND, Tick};
use editelier_media::{
    AudioBuffer, Frame, FrameData, Index, PixelFormat, ProbeInfo, Tools, VideoStream, cache,
    ffmpeg, win,
};
use editelier_render::{GpuImage, Interop, Pixels, SourceColor, SourceFrame};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

/// Video, audio and still extensions accepted at the command boundary.
pub const MEDIA_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "wmv", // video
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", // audio
    "jpg", "jpeg", "png", "heic", "heif", "bmp", "tif", "tiff", "gif", "webp", // stills
];

/// A still's default length on the timeline.
pub const STILL_SECONDS: i64 = 5;

#[derive(Clone, Debug)]
pub enum Proxy {
    None,
    Building(f32),
    Ready {
        path: PathBuf,
        video: Box<VideoStream>,
        index: Index,
    },
    Failed(String),
}

#[derive(Clone)]
pub struct MediaEntry {
    pub path: PathBuf,
    pub probe: ProbeInfo,
    /// Frame index of the first video stream.
    pub index: Option<Index>,
    pub color: SourceColor,
    pub thumb: Option<PathBuf>,
    pub proxy: Proxy,
    /// Decoded audio, 48 kHz stereo interleaved (filled in the background).
    pub audio: Option<Arc<Vec<f32>>>,
}

pub type Library = Arc<RwLock<HashMap<Id, Arc<MediaEntry>>>>;

pub fn update(lib: &Library, id: Id, f: impl FnOnce(&mut MediaEntry)) {
    let mut l = lib.write().unwrap();
    if let Some(e) = l.get(&id) {
        let mut e = (**e).clone();
        f(&mut e);
        l.insert(id, Arc::new(e));
    }
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Canonical path of an existing media file with an allowed extension (untrusted UI input).
pub fn check_media_path(raw: &str) -> ApiResult<PathBuf> {
    if raw.is_empty() || raw.len() > 4096 || raw.contains('\0') {
        return Err(ApiError::new("invalid", "That is not a file path."));
    }
    let p = std::fs::canonicalize(raw)
        .map_err(|_| ApiError::new("media_offline", format!("{raw} was moved or deleted.")))?;
    if !p.is_file() {
        return Err(ApiError::new("invalid", format!("{raw} is not a file.")));
    }
    if !MEDIA_EXTS.contains(&ext(&p).as_str()) {
        return Err(ApiError::new(
            "unsupported_codec",
            format!("{raw}: Editelier can't open this kind of file."),
        ));
    }
    Ok(p)
}

fn frame_rate(v: &VideoStream) -> Option<FrameRate> {
    let r = v.avg_fps.or(v.r_fps)?;
    let f = FrameRate::new(r.num, r.den);
    // Snap 29.97-ish averages to the exact common rate.
    Some(FrameRate::from_f64_within(f.as_f64(), 0.01).sane())
}

pub fn ticks_from_us(us: i64) -> Tick {
    Tick::from_units(us, 1_000_000)
}

/// Core metadata from a probe (+ index for an exact video duration).
pub fn meta_of(p: &ProbeInfo, index: Option<&Index>) -> MediaMeta {
    let v = p.video.first();
    let duration = match (p.kind, v, index) {
        (editelier_media::MediaKind::Image, ..) => Tick(STILL_SECONDS * TICKS_PER_SECOND),
        (_, Some(v), Some(ix)) if ix.pts.len() > 1 => {
            // Last frame end: last PTS plus the median frame interval.
            let mut d: Vec<i64> = ix.pts.windows(2).map(|w| w[1] - w[0]).collect();
            d.sort_unstable();
            let span = ix.pts[ix.pts.len() - 1] - ix.pts[0] + d[d.len() / 2];
            let tb = v.time_base;
            Tick::from_rational(span, tb.num, tb.den)
        }
        _ => ticks_from_us(p.duration_us.or(v.and_then(|v| v.duration_us)).unwrap_or(0)),
    };
    MediaMeta {
        kind: match p.kind {
            editelier_media::MediaKind::Video => MediaKind::Video,
            editelier_media::MediaKind::Audio => MediaKind::Audio,
            editelier_media::MediaKind::Image => MediaKind::Image,
        },
        duration,
        fps: v.and_then(frame_rate),
        is_vfr: v.is_some_and(|v| v.vfr),
        has_audio: !p.audio.is_empty(),
    }
}

pub fn color_of(p: &ProbeInfo) -> SourceColor {
    match p.video.first() {
        Some(v) => SourceColor::from_tags(
            v.color.matrix.as_deref(),
            v.color.range.as_deref(),
            v.height,
        ),
        None => SourceColor::from_tags(Some("bt709"), Some("pc"), 1080),
    }
}

/// Probe, index and thumbnail: everything the timeline needs right away.
pub fn load(tools: &Tools, cache_dir: &Path, path: &Path) -> ApiResult<MediaEntry> {
    let probe = editelier_media::probe(tools, path)?;
    let index = match probe.video.first() {
        Some(v) => Some(ffmpeg::index(tools, path, v, cache_dir)?),
        None => None,
    };
    let mut e = MediaEntry {
        path: path.into(),
        color: color_of(&probe),
        probe,
        index,
        thumb: None,
        proxy: Proxy::None,
        audio: None,
    };
    if e.probe.kind != editelier_media::MediaKind::Audio {
        e.thumb = thumbnail(tools, cache_dir, &e).ok();
    }
    Ok(e)
}

fn thumbnail(tools: &Tools, cache_dir: &Path, e: &MediaEntry) -> ApiResult<PathBuf> {
    let out = cache_dir.join(format!("{}.thumb.bmp", cache::cache_key(&e.path)?));
    if out.is_file() {
        return Ok(out);
    }
    let mut dec = Decoders::new(tools.clone(), None);
    let f = dec.decode(e, Tick::ZERO, false)?;
    let frame = Frame {
        pts: 0,
        width: f.width,
        height: f.height,
        format: f.format,
        data: FrameData::Cpu {
            bytes: f.bytes.clone(),
            stride: f.stride,
        },
    };
    let t = cache::thumbnail_of(&frame, 160)
        .ok_or_else(|| ApiError::new("media_error", "no thumbnail"))?;
    cache::write_atomic(&out, &bmp(t.width, t.height, &t.rgba))?;
    Ok(out)
}

/// 32-bit top-down BMP (the webview shows it; no image encoder dependency).
fn bmp(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let size = 54 + w * h * 4;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(-(h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, w * h * 4, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for p in rgba.chunks(4) {
        b.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    b
}

/// Decoded audio as 48 kHz stereo, plus cached waveform peaks.
pub fn load_audio(tools: &Tools, cache_dir: &Path, e: &MediaEntry) -> ApiResult<Arc<Vec<f32>>> {
    let a = e
        .probe
        .audio
        .first()
        .ok_or_else(|| ApiError::new("no_audio", "no audio stream"))?;
    let buf = win::decode_audio(tools, &e.path, a)?;
    let _ = cache::waveform(&e.path, cache_dir, || Ok(buf.clone()));
    Ok(Arc::new(to_stereo(&buf)))
}

fn to_stereo(b: &AudioBuffer) -> Vec<f32> {
    let ch = b.channels.max(1) as usize;
    b.samples
        .chunks(ch)
        .flat_map(|f| [f[0], *f.get(1).unwrap_or(&f[0])])
        .collect()
}

/// A decoded frame owned by the caller, ready to upload.
pub struct OwnedFrame {
    /// (PTS, from proxy): with the media id, identifies the picture.
    pub key: (i64, bool),
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes: Vec<u8>,
    pub stride: usize,
    pub color: SourceColor,
    /// Zero-copy NV12 on the render GPU (then `bytes` is empty).
    pub gpu: Option<GpuImage>,
}

impl OwnedFrame {
    pub fn source(&self) -> SourceFrame<'_> {
        let (data, stride) = (&self.bytes[..], self.stride);
        SourceFrame {
            width: self.width,
            height: self.height,
            pixels: match (&self.gpu, self.format) {
                (Some(g), _) => Pixels::Gpu(g),
                (None, PixelFormat::Nv12) => Pixels::Nv12 { data, stride },
                (None, PixelFormat::P010) => Pixels::P010 { data, stride },
                (None, PixelFormat::Rgba8) => Pixels::Rgba8 { data, stride },
            },
            color: self.color,
        }
    }
}

/// PTS of the frame shown at `t` (source time from the first frame): latest PTS <= t (D4).
pub fn pts_at(index: &Index, t: Tick) -> i64 {
    let tb = index.time_base;
    let den = tb.num as i128 * TICKS_PER_SECOND as i128;
    let off = (t.0 as i128 * tb.den as i128 + den / 2).div_euclid(den) as i64;
    index.pts[index.frame_at(index.pts[0] + off)]
}

/// Decoders for one thread (Media Foundation decoders aren't `Send`). Uses `device` for DXVA
/// decode and reads frames back to the CPU, or with [`zero_copy`](Self::zero_copy) hands NV12
/// frames to the renderer's GPU without leaving it.
pub struct Decoders {
    tools: Tools,
    device: Option<ID3D11Device>,
    interop: Option<Interop>,
    open: HashMap<PathBuf, win::VideoDecoder>,
    stills: HashMap<PathBuf, Arc<OwnedFrame>>,
}

impl Decoders {
    pub fn new(tools: Tools, device: Option<ID3D11Device>) -> Self {
        Decoders {
            tools,
            device,
            interop: None,
            open: HashMap::new(),
            stills: HashMap::new(),
        }
    }

    /// Decode on `interop`'s device and keep NV12 frames on the GPU (P010 is still read back).
    pub fn zero_copy(tools: Tools, interop: Interop) -> Self {
        Decoders {
            device: Some(interop.device().clone()),
            interop: Some(interop),
            ..Self::new(tools, None)
        }
    }

    /// The frame of media `e` at source time `t`, from its proxy when `proxy` and one is ready.
    pub fn decode(&mut self, e: &MediaEntry, t: Tick, proxy: bool) -> ApiResult<Arc<OwnedFrame>> {
        if e.probe.kind == editelier_media::MediaKind::Image {
            if let Some(s) = self.stills.get(&e.path) {
                return Ok(s.clone());
            }
            let f = win::wic::decode_image(&e.path)?;
            let still = Arc::new(own(f, e.color, false)?);
            self.stills.insert(e.path.clone(), still.clone());
            return Ok(still);
        }
        let (path, v, index, used_proxy) = match (&e.proxy, proxy) {
            (Proxy::Ready { path, video, index }, true) => (path, &**video, index, true),
            _ => (
                &e.path,
                e.probe
                    .video
                    .first()
                    .ok_or_else(|| ApiError::new("media_error", "no video stream"))?,
                e.index
                    .as_ref()
                    .ok_or_else(|| ApiError::new("media_error", "no frame index"))?,
                false,
            ),
        };
        let pts = pts_at(index, t);
        if !self.open.contains_key(path) {
            if self.open.len() >= 8 {
                self.open.clear();
            }
            match win::VideoDecoder::open(path, v, index.clone(), self.device.as_ref()) {
                Ok(mut d) => {
                    d.set_cpu_readback(self.interop.is_none());
                    self.open.insert(path.clone(), d);
                }
                Err(err) => {
                    // Royalty-free codecs Windows can't decode go through the FFmpeg helper.
                    let f = ffmpeg::frame_at(&self.tools, path, v, index, pts)
                        .map_err(|_| ApiError::from(err))?;
                    return Ok(Arc::new(own(f, e.color, used_proxy)?));
                }
            }
        }
        let d = self.open.get_mut(path).expect("inserted above");
        let mut f = d.frame_at(pts)?;
        if let FrameData::D3D11(g) = &f.data {
            if let (Some(i), PixelFormat::Nv12) = (&self.interop, f.format) {
                let img = i
                    .import(&g.texture, g.subresource, (f.width, f.height))
                    .map_err(|e| ApiError::new("media_error", e.to_string()))?;
                let (width, height) = img.size();
                return Ok(Arc::new(OwnedFrame {
                    key: (f.pts, used_proxy),
                    width,
                    height,
                    format: f.format,
                    bytes: Vec::new(),
                    stride: 0,
                    color: if used_proxy {
                        SourceColor::from_tags(Some("bt709"), Some("tv"), height)
                    } else {
                        e.color
                    },
                    gpu: Some(img),
                }));
            }
            f = d.to_cpu(&f)?;
        }
        let color = if used_proxy {
            // Proxies are encoded BT.709 limited by Media Foundation.
            SourceColor::from_tags(Some("bt709"), Some("tv"), f.height)
        } else {
            e.color
        };
        Ok(Arc::new(own(f, color, used_proxy)?))
    }
}

fn own(f: Frame, color: SourceColor, proxy: bool) -> ApiResult<OwnedFrame> {
    let FrameData::Cpu { bytes, stride } = f.data else {
        return Err(ApiError::new("media_error", "frame is not on the CPU"));
    };
    Ok(OwnedFrame {
        key: (f.pts, proxy),
        width: f.width,
        height: f.height,
        format: f.format,
        bytes,
        stride,
        color,
        gpu: None,
    })
}

/// The D3D11 device for a decode thread (DXVA), or `None` to decode in software.
pub fn decode_device() -> Option<ID3D11Device> {
    win::create_device().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use editelier_media::Rational;

    #[test]
    fn pts_lookup_is_sample_and_hold() {
        let ix = Index {
            stream: 0,
            time_base: Rational::new(1, 30000),
            pts: (0..10).map(|i| 1001 + i * 1001).collect(),
            keyframes: vec![1001],
        };
        let fps = FrameRate::FPS_29_97;
        assert_eq!(pts_at(&ix, Tick::ZERO), 1001);
        assert_eq!(pts_at(&ix, fps.tick_of(3)), 4004);
        assert_eq!(pts_at(&ix, fps.tick_of(3) - Tick(1)), 4004);
        assert_eq!(pts_at(&ix, fps.tick_of(99)), 10010);
    }

    #[test]
    fn rejects_bad_media_paths() {
        assert!(check_media_path("").is_err());
        assert!(check_media_path("C:\\definitely\\missing.mp4").is_err());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            check_media_path(exe.to_str().unwrap()).unwrap_err().code,
            "unsupported_codec"
        );
    }
}
