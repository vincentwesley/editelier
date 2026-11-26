//! Media input for Editelier: probe, PTS/keyframe index, frame-exact decode, audio decode,
//! thumbnails, waveform peaks and proxies.
//!
//! Legal split (D3): H.264, HEVC and AAC are decoded and encoded only by Windows Media Foundation
//! (`win` module). FFmpeg (LGPL build, run as a helper process) probes, indexes and decodes only the
//! royalty-free codecs in [`ffmpeg::ROYALTY_FREE`]. Public types here are platform-free.

use std::fmt;
use std::path::{Path, PathBuf};

pub mod cache;
pub mod ffmpeg;
#[cfg(windows)]
pub mod win;

pub use cache::{Thumbnail, Waveform, cache_key};
pub use ffmpeg::Tools;

/// Result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors carry a plain-language message the UI can show.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// An external tool (ffprobe/ffmpeg) failed or is missing.
    Tool(String),
    /// The file or codec can't be handled; the message says how to fix it where possible.
    Unsupported(String),
    /// Decoding or encoding failed.
    Codec(String),
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "file error: {e}"),
            Error::Tool(m) | Error::Unsupported(m) | Error::Codec(m) => f.write_str(m),
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Exact rational number (frame rates, time bases).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    /// Parses FFmpeg's `"30000/1001"` form; `None` for `0/0` or garbage.
    pub fn parse(s: &str) -> Option<Self> {
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let (num, den) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
        (num != 0 && den != 0).then_some(Self { num, den })
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// `value` in units of `self` (a time base) rescaled to units of `to`, rounding to nearest
    /// (ties away from zero).
    pub fn rescale(self, value: i64, to: Rational) -> i64 {
        let n = value as i128 * self.num as i128 * to.den as i128;
        let d = self.den as i128 * to.num as i128;
        let q = (2 * n + n.signum() * d.abs()) / (2 * d);
        q as i64
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

/// Microseconds, FFmpeg's `AV_TIME_BASE`.
pub const MICROS: Rational = Rational::new(1, 1_000_000);
/// Media Foundation's 100 ns unit.
pub const HNS: Rational = Rational::new(1, 10_000_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hdr {
    Hlg,
    Pq,
}

/// Colour tags as FFmpeg names them (`bt709`, `bt2020nc`, `arib-std-b67`, `tv`/`pc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub matrix: Option<String>,
    pub range: Option<String>,
    pub hdr: Option<Hdr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoStream {
    /// Container stream index.
    pub index: u32,
    pub codec: String,
    pub pix_fmt: Option<String>,
    pub width: u32,
    pub height: u32,
    pub time_base: Rational,
    pub avg_fps: Option<Rational>,
    pub r_fps: Option<Rational>,
    /// Variable frame rate, detected from PTS deltas of the first frames.
    pub vfr: bool,
    /// Display rotation in degrees counter-clockwise (FFmpeg convention): 0, 90, 180 or 270.
    pub rotation: u16,
    pub color: ColorInfo,
    pub duration_us: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub duration_us: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// EXIF orientation 1..=8 (1 = as stored).
    pub orientation: u16,
    pub format: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProbeInfo {
    pub path: PathBuf,
    pub kind: MediaKind,
    /// FFmpeg format name (`mov,mp4,m4a,3gp,3g2,mj2`, `matroska,webm`) or the WIC format.
    pub container: String,
    pub duration_us: Option<i64>,
    pub video: Vec<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub image: Option<ImageInfo>,
}

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "bmp", "tif", "tiff", "gif", "webp",
];

/// Probes any supported file: stills through WIC, everything else through ffprobe.
pub fn probe(tools: &Tools, path: &Path) -> Result<ProbeInfo> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        #[cfg(windows)]
        return win::wic::probe_image(path);
        #[cfg(not(windows))]
        return Err(Error::Unsupported("still images need Windows (WIC)".into()));
    }
    ffmpeg::probe(tools, path)
}

/// True when D4 says to make a proxy: larger than 1080p, keyframes further apart than 1 s, or VFR.
pub fn needs_proxy(v: &VideoStream, index: &Index) -> bool {
    let one_sec = Rational::new(1, 1).rescale(1, index.time_base);
    v.width.max(v.height) > 1920
        || v.width.min(v.height) > 1080
        || v.vfr
        || index
            .keyframes
            .windows(2)
            .any(|w| w[1] - w[0] > one_sec + one_sec / 100)
}

/// VFR if more than 2 % of PTS deltas differ from the median by more than container rounding
/// (1.5 time-base ticks) and 0.5 % of a frame. Input is presentation-sorted PTS.
pub fn detect_vfr(sorted_pts: &[i64]) -> bool {
    let mut d: Vec<i64> = sorted_pts
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|&x| x > 0)
        .collect();
    if d.len() < 3 {
        return false;
    }
    let deltas = d.clone();
    d.sort_unstable();
    let median = d[d.len() / 2] as f64;
    let tol = (median * 0.005).max(1.5);
    let off = deltas
        .iter()
        .filter(|&&x| (x as f64 - median).abs() > tol)
        .count();
    off * 50 > deltas.len()
}

/// Per-video-stream frame index: presentation-ordered PTS and keyframe PTS, in `time_base`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub stream: u32,
    pub time_base: Rational,
    pub pts: Vec<i64>,
    pub keyframes: Vec<i64>,
}

impl Index {
    /// Position of the latest frame with PTS <= `pts` (sample-and-hold), clamped to frame 0.
    pub fn frame_at(&self, pts: i64) -> usize {
        self.pts.partition_point(|&p| p <= pts).saturating_sub(1)
    }

    /// Latest keyframe PTS <= `pts` (the first keyframe if none).
    pub fn keyframe_before(&self, pts: i64) -> i64 {
        let i = self.keyframes.partition_point(|&k| k <= pts);
        self.keyframes[i.saturating_sub(1)]
    }

    /// True if a keyframe lies in `(after, upto]`.
    pub fn keyframe_between(&self, after: i64, upto: i64) -> bool {
        let i = self.keyframes.partition_point(|&k| k <= after);
        self.keyframes.get(i).is_some_and(|&k| k <= upto)
    }

    /// PTS of the frame shown at `micros` from the start of the stream.
    pub fn pts_at_micros(&self, micros: i64) -> i64 {
        let first = self.pts.first().copied().unwrap_or(0);
        self.pts[self.frame_at(first + MICROS.rescale(micros, self.time_base))]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 8-bit 4:2:0, Y plane then interleaved UV plane.
    Nv12,
    /// 10-bit in the high bits of 16-bit words, same layout as NV12.
    P010,
    /// 8-bit RGBA, straight alpha.
    Rgba8,
}

/// Decoded frame. `Cpu` buffers have `height` Y rows then `height / 2` UV rows (NV12/P010) or
/// `height` RGBA rows, each `stride` bytes.
pub struct Frame {
    /// PTS in the stream time base.
    pub pts: i64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: FrameData,
}

pub enum FrameData {
    Cpu {
        bytes: Vec<u8>,
        stride: usize,
    },
    /// Decoder surface on the caller's D3D11 device. Kept alive (out of the decoder pool) for as
    /// long as this frame lives.
    #[cfg(windows)]
    D3D11(win::GpuFrame),
}

impl Frame {
    /// The Y (luma) plane, or `None` for GPU/RGBA frames. P010 returns the high byte per sample.
    pub fn luma(&self) -> Option<Vec<u8>> {
        let FrameData::Cpu { bytes, stride } = &self.data else {
            return None;
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = Vec::with_capacity(w * h);
        for row in bytes.chunks(*stride).take(h) {
            match self.format {
                PixelFormat::Nv12 => out.extend_from_slice(&row[..w]),
                PixelFormat::P010 => out.extend((0..w).map(|x| row[2 * x + 1])),
                PixelFormat::Rgba8 => return None,
            }
        }
        Some(out)
    }

    /// RGBA8 copy of a CPU frame (BT.601 for <= 576 lines, else BT.709; limited range).
    /// The GPU path does this properly in the render crate; this is for thumbnails and tests.
    pub fn to_rgba(&self) -> Option<Vec<u8>> {
        let FrameData::Cpu { bytes, stride } = &self.data else {
            return None;
        };
        let (w, h, s) = (self.width as usize, self.height as usize, *stride);
        if self.format == PixelFormat::Rgba8 {
            return Some(
                bytes
                    .chunks(s)
                    .take(h)
                    .flat_map(|r| &r[..w * 4])
                    .copied()
                    .collect(),
            );
        }
        let (kr, kb) = if h <= 576 {
            (0.299, 0.114)
        } else {
            (0.2126, 0.0722)
        };
        let kg = 1.0 - kr - kb;
        let px = |row: &[u8], x: usize| -> f32 {
            match self.format {
                PixelFormat::P010 => {
                    u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) as f32 / 256.0
                }
                _ => row[x] as f32,
            }
        };
        let uv_off = s * h;
        let mut out = vec![255u8; w * h * 4];
        for y in 0..h {
            let yrow = &bytes[y * s..];
            let crow = &bytes[uv_off + (y / 2) * s..];
            for x in 0..w {
                let luma = (px(yrow, x) - 16.0) * (255.0 / 219.0);
                let cb = (px(crow, x & !1) - 128.0) * (255.0 / 224.0);
                let cr = (px(crow, (x & !1) + 1) - 128.0) * (255.0 / 224.0);
                let r = luma + 2.0 * (1.0 - kr) * cr;
                let b = luma + 2.0 * (1.0 - kb) * cb;
                let g = (luma - kr * r - kb * b) / kg;
                let o = (y * w + x) * 4;
                out[o] = r.clamp(0.0, 255.0) as u8;
                out[o + 1] = g.clamp(0.0, 255.0) as u8;
                out[o + 2] = b.clamp(0.0, 255.0) as u8;
            }
        }
        Some(out)
    }
}

/// Interleaved f32 audio at 48 kHz.
#[derive(Clone, Debug, Default)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u32,
    pub samples: Vec<f32>,
}

pub(crate) fn f32_le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

/// Output sample rate for all decoded audio.
pub const SAMPLE_RATE: u32 = 48_000;

/// Resamples interleaved audio to 48 kHz (rubato FFT resampler; no-op if already 48 kHz).
pub fn resample_to_48k(input: AudioBuffer) -> Result<AudioBuffer> {
    use rubato::audioadapter_buffers::direct::InterleavedSlice;
    use rubato::{Fft, FixedSync, Resampler};
    if input.sample_rate == SAMPLE_RATE || input.samples.is_empty() {
        return Ok(AudioBuffer {
            sample_rate: SAMPLE_RATE,
            ..input
        });
    }
    let ch = input.channels as usize;
    let frames = input.samples.len() / ch;
    let err = |e: &dyn fmt::Display| Error::Codec(format!("resample failed: {e}"));
    let mut r = Fft::<f32>::new(
        input.sample_rate as usize,
        SAMPLE_RATE as usize,
        1024,
        ch,
        FixedSync::Both,
    )
    .map_err(|e| err(&e))?;
    let src = InterleavedSlice::new(&input.samples, ch, frames).map_err(|e| err(&e))?;
    let out = r.process_all(&src, frames, None).map_err(|e| err(&e))?;
    Ok(AudioBuffer {
        sample_rate: SAMPLE_RATE,
        channels: input.channels,
        samples: out.take_data(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_parse_and_rescale() {
        assert_eq!(
            Rational::parse("30000/1001"),
            Some(Rational::new(30000, 1001))
        );
        assert_eq!(Rational::parse("0/0"), None);
        // 1001 ticks at 1/30000 = 333_667 hns (33.3667 ms, rounded)
        assert_eq!(Rational::new(1, 30000).rescale(1001, HNS), 333_667);
        assert_eq!(Rational::new(1, 30000).rescale(-1001, HNS), -333_667);
    }

    #[test]
    fn vfr_detection() {
        let cfr: Vec<i64> = (0..300).map(|i| i * 1001).collect();
        assert!(!detect_vfr(&cfr));
        // MKV millisecond rounding of 29.97 fps (33/34 ms) is still CFR
        let mkv: Vec<i64> = (0..300)
            .map(|i| (i as f64 * 33.3667).round() as i64)
            .collect();
        assert!(!detect_vfr(&mkv));
        let mut vfr = cfr.clone();
        for (i, p) in vfr.iter_mut().enumerate() {
            *p += (i as i64 * 7919) % 400;
        }
        vfr.sort_unstable();
        assert!(detect_vfr(&vfr));
    }

    #[test]
    fn index_lookup() {
        let ix = Index {
            stream: 0,
            time_base: Rational::new(1, 30),
            pts: (0..10).collect(),
            keyframes: vec![0, 5],
        };
        assert_eq!(ix.frame_at(-3), 0);
        assert_eq!(ix.frame_at(7), 7);
        assert_eq!(ix.frame_at(99), 9);
        assert_eq!(ix.keyframe_before(4), 0);
        assert_eq!(ix.keyframe_before(5), 5);
        assert!(ix.keyframe_between(4, 5));
        assert!(!ix.keyframe_between(5, 9));
        assert_eq!(ix.pts_at_micros(100_000), 3);
    }
}
