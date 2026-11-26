//! MP4 export for Editelier.
//!
//! H.264/HEVC + AAC are encoded **only** through Windows Media Foundation (OS-licensed
//! codecs, D3). Types, presets and loudness are platform-free; the encoder lives in
//! [`win`] behind `cfg(windows)`.

use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod loudness;
pub mod mp4;
#[cfg(windows)]
pub mod win;

/// Audio is always AAC-LC, 48 kHz, stereo (D6). Input is f32 interleaved.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RateControl {
    Cbr {
        bitrate_kbps: u32,
    },
    Vbr {
        bitrate_kbps: u32,
        max_bitrate_kbps: u32,
    },
    /// Encoder quality 1..=100 (`CODECAPI_AVEncCommonQuality`).
    Quality {
        quality: u32,
    },
    /// Constant QP 0..=51 (`CODECAPI_AVEncVideoEncodeQP`).
    ConstQp {
        qp: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub rate_control: RateControl,
    /// Closed GOP length; must be in (0, 2] s.
    pub gop_seconds: f64,
    pub b_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AudioSettings {
    /// Requested AAC bitrate. MF's AAC encoder supports 96/128/160/192 kbps; the
    /// highest supported value not above this is used.
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct LoudnessTarget {
    pub target_lufs: f64,
    pub true_peak_dbtp: f64,
}

/// An export preset, loaded from `presets/*.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub test_only: bool,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub video: VideoSettings,
    pub audio: Option<AudioSettings>,
    pub loudness: Option<LoudnessTarget>,
    pub source_url: String,
    pub verified_on: String,
    #[serde(default)]
    pub notes: String,
}

const BUILTIN: &[&str] = &[
    include_str!("../../../presets/youtube-1080p30.json"),
    include_str!("../../../presets/youtube-1080p60.json"),
    include_str!("../../../presets/youtube-4k30.json"),
    include_str!("../../../presets/youtube-4k60.json"),
    include_str!("../../../presets/tiktok-1080x1920.json"),
    include_str!("../../../presets/instagram-reel-1080x1920.json"),
    include_str!("../../../presets/instagram-feed-1080x1350.json"),
    include_str!("../../../presets/master-1080p.json"),
    include_str!("../../../presets/master-4k.json"),
    include_str!("../../../presets/test-near-lossless.json"),
];

impl Preset {
    pub fn from_json(json: &str) -> Result<Self, ExportError> {
        let p: Preset =
            serde_json::from_str(json).map_err(|e| ExportError::InvalidInput(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }

    /// All presets shipped in `presets/` (compiled in).
    pub fn builtin() -> Vec<Preset> {
        BUILTIN
            .iter()
            .map(|j| Preset::from_json(j).expect("built-in preset is valid"))
            .collect()
    }

    pub fn builtin_by_id(id: &str) -> Option<Preset> {
        Self::builtin().into_iter().find(|p| p.id == id)
    }

    pub fn validate(&self) -> Result<(), ExportError> {
        let bad = |m: &str| {
            Err(ExportError::InvalidInput(format!(
                "preset {}: {m}",
                self.id
            )))
        };
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return bad("width/height must be even and non-zero");
        }
        if self.fps_num == 0 || self.fps_den == 0 {
            return bad("fps must be non-zero");
        }
        let gop = self.video.gop_seconds;
        if !(gop > 0.0 && gop <= 2.0) {
            return bad("gop_seconds must be in (0, 2]");
        }
        match self.video.rate_control {
            RateControl::Cbr { bitrate_kbps: 0 } => return bad("bitrate must be > 0"),
            RateControl::Vbr {
                bitrate_kbps,
                max_bitrate_kbps,
            } if bitrate_kbps == 0 || max_bitrate_kbps < bitrate_kbps => {
                return bad("vbr needs 0 < bitrate <= max_bitrate");
            }
            RateControl::Quality { quality } if !(1..=100).contains(&quality) => {
                return bad("quality must be 1..=100");
            }
            RateControl::ConstQp { qp } if qp > 51 => return bad("qp must be 0..=51"),
            _ => {}
        }
        if self.source_url.is_empty() || self.verified_on.is_empty() {
            return bad("source_url and verified_on are required");
        }
        Ok(())
    }

    /// GOP length in frames (at least 1).
    pub fn gop_frames(&self) -> u32 {
        let fps = self.fps_num as f64 / self.fps_den as f64;
        ((self.video.gop_seconds * fps).round() as u32).max(1)
    }

    /// Presentation time of frame `n` in 100 ns units (exact integer maths, zero-based).
    pub fn frame_time_hns(&self, n: u64) -> i64 {
        (n as u128 * 10_000_000 * self.fps_den as u128 / self.fps_num as u128) as i64
    }
}

/// AAC bytes/s actually used for a requested kbps (MF AAC encoder accepts 12000,
/// 16000, 20000, 24000 bytes/s).
pub fn aac_bytes_per_second(requested_kbps: u32) -> u32 {
    let wanted = requested_kbps * 1000 / 8;
    [24_000, 20_000, 16_000, 12_000]
        .into_iter()
        .find(|&b| b <= wanted)
        .unwrap_or(12_000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Vendor {
    // Order = auto-pick preference.
    Intel,
    Amd,
    Nvidia,
    OtherHardware,
    Software,
}

impl Vendor {
    /// From `MFT_ENUM_HARDWARE_VENDOR_ID_Attribute` (e.g. `VEN_8086`) or the friendly name.
    pub fn identify(vendor_id: Option<&str>, friendly_name: &str, hardware: bool) -> Vendor {
        if !hardware {
            return Vendor::Software;
        }
        let id = vendor_id.unwrap_or("").to_ascii_uppercase();
        let name = friendly_name.to_ascii_lowercase();
        if id.contains("8086") || name.contains("intel") || name.contains("quick sync") {
            Vendor::Intel
        } else if id.contains("1002") || id.contains("1022") || name.contains("amd") {
            Vendor::Amd
        } else if id.contains("10DE") || name.contains("nvidia") {
            Vendor::Nvidia
        } else {
            Vendor::OtherHardware
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderInfo {
    pub name: String,
    pub vendor: Vendor,
    pub codec: VideoCodec,
    pub hardware: bool,
    /// DXGI adapter the hardware MFT runs on.
    pub adapter_luid: Option<u64>,
}

/// Pick from an already-sorted list: the preferred vendor if present, else the first.
pub fn pick_encoder(encoders: &[EncoderInfo], prefer: Option<Vendor>) -> Option<&EncoderInfo> {
    prefer
        .and_then(|v| encoders.iter().find(|e| e.vendor == v))
        .or_else(|| encoders.first())
}

/// Shared cancel flag; clone it into the UI thread.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub frames_written: u64,
    pub total_frames: Option<u64>,
}

#[derive(Debug)]
pub enum ExportError {
    InvalidInput(String),
    Unsupported(String),
    Media(String),
    Io(std::io::Error),
    Cancelled,
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InvalidInput(m) => write!(f, "invalid input: {m}"),
            ExportError::Unsupported(m) => write!(f, "unsupported: {m}"),
            ExportError::Media(m) => write!(f, "media foundation: {m}"),
            ExportError::Io(e) => write!(f, "io: {e}"),
            ExportError::Cancelled => write!(f, "export cancelled"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e)
    }
}

/// RGBA8 (sRGB-encoded, full range) to NV12 BT.709 limited range, tightly packed.
pub fn rgba_to_nv12(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, ExportError> {
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) || rgba.len() != width * height * 4 {
        return Err(ExportError::InvalidInput(
            "rgba buffer must be width*height*4 with even dimensions".into(),
        ));
    }
    let mut out = vec![0u8; width * height * 3 / 2];
    let (y_plane, uv_plane) = out.split_at_mut(width * height);
    // BT.709: Kr = 0.2126, Kb = 0.0722; limited range Y 16..235, C 16..240.
    let px = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 3];
        (p[0] as f32, p[1] as f32, p[2] as f32)
    };
    let luma = |(r, g, b): (f32, f32, f32)| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    for (i, y) in y_plane.iter_mut().enumerate() {
        *y = (16.0 + luma(px(i)) * 219.0 / 255.0).round() as u8;
    }
    for cy in 0..height / 2 {
        for cx in 0..width / 2 {
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px((cy * 2 + dy) * width + cx * 2 + dx);
                r += p.0 / 4.0;
                g += p.1 / 4.0;
                b += p.2 / 4.0;
            }
            let y = luma((r, g, b));
            let cb = (b - y) / 1.8556;
            let cr = (r - y) / 1.5748;
            let o = cy * width + cx * 2;
            uv_plane[o] = (128.0 + cb * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
            uv_plane[o + 1] = (128.0 + cr * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_presets_parse_and_validate() {
        let all = Preset::builtin();
        assert_eq!(all.len(), BUILTIN.len());
        for p in &all {
            assert!(p.source_url.starts_with("https://"), "{}", p.id);
            assert!(p.gop_frames() as f64 / (p.fps_num as f64 / p.fps_den as f64) <= 2.0);
        }
        let yt = Preset::builtin_by_id("youtube-1080p30").unwrap();
        assert_eq!(yt.gop_frames(), 15);
        assert_eq!(yt.frame_time_hns(30), 10_000_000);
    }

    #[test]
    fn invalid_preset_rejected() {
        let mut j: serde_json::Value =
            serde_json::from_str(include_str!("../../../presets/youtube-1080p30.json")).unwrap();
        j["video"]["gop_seconds"] = 3.0.into();
        assert!(Preset::from_json(&j.to_string()).is_err());
    }

    #[test]
    fn ntsc_timestamps_are_exact() {
        let mut p = Preset::builtin_by_id("youtube-1080p30").unwrap();
        (p.fps_num, p.fps_den) = (30_000, 1001);
        assert_eq!(p.frame_time_hns(30_000), 1001 * 10_000_000);
    }

    #[test]
    fn aac_bitrate_clamps_to_mf_values() {
        assert_eq!(aac_bytes_per_second(384), 24_000);
        assert_eq!(aac_bytes_per_second(160), 20_000);
        assert_eq!(aac_bytes_per_second(64), 12_000);
    }

    #[test]
    fn vendor_identification_and_pick_order() {
        assert_eq!(Vendor::identify(Some("VEN_8086"), "", true), Vendor::Intel);
        assert_eq!(Vendor::identify(None, "AMDh264Encoder", true), Vendor::Amd);
        assert_eq!(Vendor::identify(Some("VEN_10DE"), "", true), Vendor::Nvidia);
        assert_eq!(
            Vendor::identify(Some("VEN_8086"), "", false),
            Vendor::Software
        );
        let mk = |vendor| EncoderInfo {
            name: String::new(),
            vendor,
            codec: VideoCodec::H264,
            hardware: vendor != Vendor::Software,
            adapter_luid: None,
        };
        let mut list = vec![mk(Vendor::Software), mk(Vendor::Nvidia), mk(Vendor::Intel)];
        list.sort_by_key(|e| e.vendor);
        assert_eq!(pick_encoder(&list, None).unwrap().vendor, Vendor::Intel);
        assert_eq!(
            pick_encoder(&list, Some(Vendor::Nvidia)).unwrap().vendor,
            Vendor::Nvidia
        );
        assert_eq!(
            pick_encoder(&list, Some(Vendor::Amd)).unwrap().vendor,
            Vendor::Intel
        );
    }

    #[test]
    fn rgba_to_nv12_reference_colours() {
        // 2x2 white, black, pure red: BT.709 limited.
        let solid = |r, g, b| {
            let px = [r, g, b, 255u8];
            rgba_to_nv12(&px.repeat(4), 2, 2).unwrap()
        };
        assert_eq!(solid(255, 255, 255), vec![235, 235, 235, 235, 128, 128]);
        assert_eq!(solid(0, 0, 0), vec![16, 16, 16, 16, 128, 128]);
        // Red: Y = 16 + 0.2126*219 = 62.6 -> 63, Cb = 102, Cr = 240.
        assert_eq!(solid(255, 0, 0), vec![63, 63, 63, 63, 102, 240]);
        assert!(rgba_to_nv12(&[0; 12], 3, 1).is_err());
    }
}
