//! Media the open project uses: probe results, frame indexes, thumbnails, proxies and decoded
//! audio, plus the per-thread decoder pool that turns (media, source time) into a frame.

use crate::api::{ApiError, ApiResult};
use crate::mix;
use editelier_core::{FrameRate, Id, MediaKind, MediaMeta, TICKS_PER_SECOND, Tick};
use editelier_media::{
    AudioBuffer, Frame, FrameData, Index, PixelFormat, ProbeInfo, Rational, Tools, VideoStream,
    cache, ffmpeg, win,
};
use editelier_render::{GpuImage, Interop, Orient, Pixels, SourceColor, SourceFrame};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf, Prefix};
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
    /// Decoded audio on disk (filled in the background).
    pub audio: Option<Arc<AudioPcm>>,
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

/// A media path read from a project file (untrusted): local disk only. The shape is checked on
/// the raw string before any filesystem call, because merely touching `\\host\x` (or WebDAV
/// `\\host@SSL\x`) sends the user's NTLM hash and IP to that host. A canonical path that turns
/// out to be a share (mapped network drive) is refused too.
pub fn check_project_media_path(raw: &str) -> ApiResult<PathBuf> {
    local_shape(raw)?;
    let p = check_media_path(raw)?;
    if matches!(p.components().next(), Some(Component::Prefix(x))
        if matches!(x.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)))
    {
        return Err(not_local(raw));
    }
    Ok(p)
}

/// String-only check (no filesystem access): `C:\...` with a media extension.
fn local_shape(raw: &str) -> ApiResult<()> {
    let p = Path::new(raw);
    let disk = matches!(p.components().next(), Some(Component::Prefix(x))
        if matches!(x.kind(), Prefix::Disk(_)));
    if raw.starts_with(r"\\")
        || raw.starts_with("//")
        || raw.len() > 4096
        || raw.contains('\0')
        || !p.is_absolute()
        || !disk
    {
        return Err(not_local(raw));
    }
    if !MEDIA_EXTS.contains(&ext(p).as_str()) {
        return Err(ApiError::new(
            "unsupported_codec",
            format!("{raw}: Editelier can't open this kind of file."),
        ));
    }
    Ok(())
}

fn not_local(raw: &str) -> ApiError {
    ApiError::new("media_offline", format!("{raw} isn't on this PC's drives."))
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
    let (w, h, rgba) = orient_rgba(t.width, t.height, &t.rgba, f.orient);
    cache::write_atomic(&out, &bmp(w, h, &rgba))?;
    Ok(out)
}

/// `rgba` (`w` x `h`, stored orientation) turned upright: display size and pixels. Same mapping
/// as the compositor's (render crate `orient_uv`), in pixels.
fn orient_rgba(w: u32, h: u32, rgba: &[u8], o: Orient) -> (u32, u32, Vec<u8>) {
    let (dw, dh) = o.size((w, h));
    let (w1, h1) = (dw - 1, dh - 1);
    let mut out = Vec::with_capacity(rgba.len());
    for y in 0..dh {
        for x in 0..dw {
            let (sx, sy) = match o.0 {
                2 => (w1 - x, y),
                3 => (w1 - x, h1 - y),
                4 => (x, h1 - y),
                5 => (y, x),
                6 => (y, w1 - x),
                7 => (h1 - y, w1 - x),
                8 => (h1 - y, x),
                _ => (x, y),
            };
            let i = (sy * w + sx) as usize * 4;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    (dw, dh, out)
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

/// Decoded audio in the disk cache: 48 kHz stereo f32 LE, frame 0 at the clip's source time 0
/// (the first video frame, or the file's start for audio-only media). Read in ranges, so memory
/// stays bounded however long the clip is.
pub struct AudioPcm {
    file: std::fs::File,
    pub frames: i64,
}

impl AudioPcm {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let frames = (file.metadata()?.len() / 8) as i64;
        Ok(Self { file, frames })
    }

    /// Stereo frames starting at frame `from` into `out` (interleaved); outside the audio, and on
    /// a read error, the samples are silence.
    pub fn read(&self, from: i64, out: &mut [f32]) {
        use std::os::windows::fs::FileExt;
        out.fill(0.0);
        let n = (out.len() / 2) as i64;
        let (lo, hi) = (from.max(0), (from + n).min(self.frames));
        if hi <= lo {
            return;
        }
        let dst = &mut out[((lo - from) * 2) as usize..((hi - from) * 2) as usize];
        let mut bytes = vec![0u8; dst.len() * 4];
        let mut got = 0;
        while got < bytes.len() {
            match self
                .file
                .seek_read(&mut bytes[got..], (lo * 8) as u64 + got as u64)
            {
                Ok(0) | Err(_) => break,
                Ok(k) => got += k,
            }
        }
        for (d, b) in dst.iter_mut().zip(bytes[..got].as_chunks::<4>().0) {
            *d = f32::from_le_bytes(*b);
        }
    }
}

fn io(e: std::io::Error) -> ApiError {
    editelier_media::Error::Io(e).into()
}

/// Streams the audio to the disk cache (48 kHz stereo, aligned to source time 0) and writes the
/// waveform peaks on the way; reuses the cache when present.
pub fn load_audio(tools: &Tools, cache_dir: &Path, e: &MediaEntry) -> ApiResult<Arc<AudioPcm>> {
    let a = e
        .probe
        .audio
        .first()
        .ok_or_else(|| ApiError::new("no_audio", "no audio stream"))?;
    let file = cache_dir.join(format!("{}.a{}.pcm", cache::cache_key(&e.path)?, a.index));
    if !file.is_file() {
        std::fs::create_dir_all(cache_dir).map_err(io)?;
        // Decoded sample 0 is file time 0; source time 0 is the first video frame (D4).
        let start = e.index.as_ref().map_or(0, |ix| {
            ix.time_base.rescale(ix.pts[0], Rational::new(1, mix::RATE))
        });
        let part = file.with_extension("part");
        let mut w = std::io::BufWriter::new(std::fs::File::create(&part).map_err(io)?);
        let mut skip = start.max(0);
        let mut window = (0usize, 0f32, 0f32);
        let mut peaks = Vec::new();
        let mut frame = |w: &mut dyn std::io::Write, l: f32, r: f32| -> std::io::Result<()> {
            w.write_all(&l.to_le_bytes())?;
            w.write_all(&r.to_le_bytes())?;
            window = (window.0 + 1, window.1.min(l).min(r), window.2.max(l).max(r));
            if window.0 == 480 {
                peaks.extend([window.1, window.2]);
                window = (0, 0.0, 0.0);
            }
            Ok(())
        };
        let res = (|| -> ApiResult<()> {
            for _ in 0..(-start).max(0) {
                frame(&mut w, 0.0, 0.0).map_err(io)?;
            }
            win::stream_audio(tools, &e.path, a, &mut |ch, s| {
                for f in s.chunks_exact(ch.max(1) as usize) {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    frame(&mut w, f[0], *f.get(1).unwrap_or(&f[0]))?;
                }
                Ok(())
            })?;
            use std::io::Write;
            w.flush().map_err(io)?;
            Ok(())
        })();
        drop(w);
        if let Err(err) = res {
            let _ = std::fs::remove_file(&part);
            return Err(err);
        }
        if window.0 > 0 {
            peaks.extend([window.1, window.2]);
        }
        std::fs::rename(&part, &file).map_err(io)?;
        // Two samples (min, max) per 10 ms block at a nominal 200 Hz: peaks_of keeps them as is.
        let _ = cache::waveform(&e.path, cache_dir, || {
            Ok(AudioBuffer {
                sample_rate: 200,
                channels: 1,
                samples: peaks,
            })
        });
    }
    Ok(Arc::new(AudioPcm::open(&file).map_err(io)?))
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
    pub orient: Orient,
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
            orient: self.orient,
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
            // WIC already turned the still upright.
            let still = Arc::new(own(f, e.color, false, Orient::default())?);
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
        // Proxies are encoded from unrotated decoder output, so both carry the source rotation.
        let orient = Orient(e.probe.video.first().map_or(1, |v| v.orientation()));
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
                    return Ok(Arc::new(own(f, e.color, used_proxy, orient)?));
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
                    orient,
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
        Ok(Arc::new(own(f, color, used_proxy, orient)?))
    }
}

fn own(f: Frame, color: SourceColor, proxy: bool, orient: Orient) -> ApiResult<OwnedFrame> {
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
        orient,
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

    fn fixture(name: &str) -> Option<(Tools, PathBuf, PathBuf)> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test-media")
            .join(name);
        let Ok(tools) = Tools::locate() else {
            eprintln!("SKIP: no FFmpeg helper");
            return None;
        };
        if !p.is_file() {
            eprintln!(
                "SKIP: {} missing; run scripts/make-test-media.ps1",
                p.display()
            );
            return None;
        }
        let cache = std::env::temp_dir()
            .join("editelier-library-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&cache);
        Some((tools, p, cache))
    }

    #[test]
    fn phone_clip_shows_upright() {
        use editelier_render::{Fonts, Gpu, Renderer};
        let Some((tools, path, cache)) = fixture("phone_rot90cw_h264.mp4") else {
            return;
        };
        let e = load(&tools, &cache, &path).unwrap();
        let v = &e.probe.video[0];
        assert_eq!(v.display_size(), (360, 640));
        let Ok(gpu) = Gpu::headless() else {
            return eprintln!("SKIP: no GPU");
        };
        // Zero-copy when the adapter supports it (`EDITELIER_ZERO_COPY=0` forces the CPU path).
        let mut dec = match Interop::new(&gpu) {
            Ok(i) => Decoders::zero_copy(tools, i),
            Err(_) => Decoders::new(tools, None),
        };
        let f = dec.decode(&e, Tick(TICKS_PER_SECOND / 2), false).unwrap();
        assert_eq!((f.width, f.height, f.orient), (640, 360, Orient(6)));
        if std::env::var_os("EDITELIER_ZERO_COPY").is_some_and(|v| v == "1") {
            assert!(f.gpu.is_some(), "zero-copy path not taken");
        }
        let mut r = Renderer::new(&gpu, Fonts::bundled());
        r.upload(&f.source()).unwrap();
        let (w, h) = v.display_size();
        r.compose((w, h), &[], 1.0);
        let out = r.read_rgba8().unwrap();
        // Stored marker: red top-left, lime top-right, blue bottom-left. Turned 90 degrees
        // clockwise: blue top-left, red top-right, lime bottom-right.
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            match (out[i] > 150, out[i + 1] > 150, out[i + 2] > 150) {
                (true, false, false) => 'R',
                (false, true, false) => 'G',
                (false, false, true) => 'B',
                _ => 'X',
            }
        };
        let got: String = [
            (w / 8, h / 8),
            (w * 7 / 8, h / 8),
            (w / 8, h * 7 / 8),
            (w * 7 / 8, h * 7 / 8),
        ]
        .iter()
        .map(|&(x, y)| at(x, y))
        .collect();
        assert_eq!(got, "BRXG");
        // The thumbnail is upright too.
        let bmp = std::fs::read(e.thumb.unwrap()).unwrap();
        let (tw, th) = (
            i32::from_le_bytes(bmp[18..22].try_into().unwrap()),
            -i32::from_le_bytes(bmp[22..26].try_into().unwrap()),
        );
        assert!(th > tw, "thumbnail {tw}x{th}");
    }

    #[test]
    fn audio_lines_up_with_first_video_frame() {
        let Some((tools, path, cache)) = fixture("av_offset_h264_aac.mp4") else {
            return;
        };
        let e = load(&tools, &cache, &path).unwrap();
        let a = load_audio(&tools, &cache, &e).unwrap();
        let mut buf = vec![0f32; 96_000 * 2];
        a.read(0, &mut buf);
        let onset = buf.chunks(2).position(|f| f[0].abs() > 0.25).unwrap();
        // Burst at 1.000 s of source time (reaches 0.25 ~4 samples in); 1 ms = 48 samples.
        assert!((onset as i64 - 48_004).abs() <= 48, "onset {onset}");
        // Cached: a second load reads the same file.
        let again = load_audio(&tools, &cache, &e).unwrap();
        assert_eq!(again.frames, a.frames);
        assert!(cache::waveform(&path, &cache, || panic!("waveform not cached")).is_ok());
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

    #[test]
    fn project_media_paths_are_local_only() {
        let dir = std::env::temp_dir().join("editelier-local-path-test");
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("a.mp4");
        std::fs::write(&ok, b"x").unwrap();
        let ok = plain_str(&ok);
        for bad in [
            r"\\host\share\a.mp4",
            r"\\?\UNC\host\share\a.mp4",
            r"\\host@SSL\x\a.mp4",
            r"\\host@80\x\a.mp4",
            "//host/x.mp4",
            r"media\a.mp4",
            r"\a.mp4",
        ] {
            // String-only rejection: no filesystem call ever reaches the host.
            assert_eq!(local_shape(bad).unwrap_err().code, "media_offline", "{bad}");
            assert!(check_project_media_path(bad).is_err(), "{bad}");
        }
        assert!(local_shape(r"C:\x\a.exe").is_err());
        assert!(check_project_media_path(&ok).is_ok(), "{ok}");
    }

    fn plain_str(p: &Path) -> String {
        crate::commands::plain(std::fs::canonicalize(p).unwrap())
            .display()
            .to_string()
    }
}
