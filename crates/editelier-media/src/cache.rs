//! Disk cache: key per source file, thumbnails and waveform peaks.

use crate::{AudioBuffer, Frame, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// FNV-1a 64: stable across Rust versions, unlike `DefaultHasher`.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Cache key from size, mtime and the first and last 64 KiB of the file (hex).
/// ponytail: sampled hash, not a full-file hash; collides only for same-size files edited in the
/// middle within the same mtime tick.
pub fn cache_key(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = fnv(0xcbf2_9ce4_8422_2325, &meta.len().to_le_bytes());
    h = fnv(h, &mtime.to_le_bytes());
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf)?;
    h = fnv(h, &buf[..n]);
    if meta.len() > buf.len() as u64 {
        f.seek(SeekFrom::End(-(buf.len() as i64)))?;
        f.read_exact(&mut buf)?;
        h = fnv(h, &buf);
    }
    Ok(format!("{h:016x}"))
}

/// Writes via a temp file + rename so readers never see a half-written cache entry.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Small RGBA image at `pts` (stream time base).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub pts: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Box-filter downscale of a CPU frame to `width` (height keeps aspect, even).
pub fn thumbnail_of(frame: &Frame, width: u32) -> Option<Thumbnail> {
    let src = frame.to_rgba()?;
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let w = (width as usize).min(sw).max(1);
    let h = ((sh * w / sw).max(2)) & !1;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1));
            let mut acc = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let o = (sy * sw + sx) * 4;
                    for c in 0..4 {
                        acc[c] += src[o + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (y * w + x) * 4;
            for c in 0..4 {
                rgba[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    Some(Thumbnail {
        pts: frame.pts,
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

/// Thumbnails every `every_us` microseconds, cached in `cache_dir`. `frame_at(micros)` supplies a
/// CPU frame for that time (the caller picks the decoder).
pub fn thumbnails(
    path: &Path,
    cache_dir: &Path,
    duration_us: i64,
    every_us: i64,
    width: u32,
    mut frame_at: impl FnMut(i64) -> Result<Frame>,
) -> Result<Vec<Thumbnail>> {
    let file = cache_dir.join(format!(
        "{}.thumbs-{every_us}-{width}.bin",
        cache_key(path)?
    ));
    if let Ok(bytes) = std::fs::read(&file)
        && let Some(t) = decode_thumbs(&bytes)
    {
        return Ok(t);
    }
    let mut out = Vec::new();
    let mut t = 0;
    while t < duration_us.max(1) {
        let frame = frame_at(t)?;
        out.extend(thumbnail_of(&frame, width));
        t += every_us.max(1);
    }
    let mut bytes = Vec::new();
    for th in &out {
        bytes.extend_from_slice(&th.pts.to_le_bytes());
        bytes.extend_from_slice(&th.width.to_le_bytes());
        bytes.extend_from_slice(&th.height.to_le_bytes());
        bytes.extend_from_slice(&th.rgba);
    }
    write_atomic(&file, &bytes)?;
    Ok(out)
}

fn decode_thumbs(mut b: &[u8]) -> Option<Vec<Thumbnail>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let pts = i64::from_le_bytes(b.get(..8)?.try_into().ok()?);
        let width = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?);
        let height = u32::from_le_bytes(b.get(12..16)?.try_into().ok()?);
        let n = width as usize * height as usize * 4;
        let rgba = b.get(16..16 + n)?.to_vec();
        b = &b[16 + n..];
        out.push(Thumbnail {
            pts,
            width,
            height,
            rgba,
        });
    }
    Some(out)
}

/// Min/max peaks per 10 ms (480 frames at 48 kHz), all channels mixed into one envelope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waveform {
    pub peaks: Vec<(f32, f32)>,
}

pub fn peaks_of(audio: &AudioBuffer) -> Waveform {
    let block = (audio.sample_rate / 100).max(1) as usize * audio.channels.max(1) as usize;
    Waveform {
        peaks: audio
            .samples
            .chunks(block)
            .map(|c| {
                c.iter()
                    .fold((0f32, 0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)))
            })
            .collect(),
    }
}

/// Waveform peaks for `path`, cached in `cache_dir`; `decode` supplies the audio on a miss.
pub fn waveform(
    path: &Path,
    cache_dir: &Path,
    decode: impl FnOnce() -> Result<AudioBuffer>,
) -> Result<Waveform> {
    let file = cache_dir.join(format!("{}.peaks.bin", cache_key(path)?));
    if let Ok(bytes) = std::fs::read(&file) {
        let f = crate::f32_le(&bytes);
        return Ok(Waveform {
            peaks: f.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect(),
        });
    }
    let w = peaks_of(&decode()?);
    let bytes: Vec<u8> = w
        .peaks
        .iter()
        .flat_map(|(a, b)| a.to_le_bytes().into_iter().chain(b.to_le_bytes()))
        .collect();
    write_atomic(&file, &bytes)?;
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameData, PixelFormat};

    #[test]
    fn peaks_per_10ms() {
        let audio = AudioBuffer {
            sample_rate: 48_000,
            channels: 2,
            samples: (0..48_000 * 2)
                .map(|i| if i % 2 == 0 { 0.5 } else { -0.25 })
                .collect(),
        };
        let w = peaks_of(&audio);
        assert_eq!(w.peaks.len(), 100);
        assert_eq!(w.peaks[0], (-0.25, 0.5));
    }

    #[test]
    fn thumbnail_downscales_rgba() {
        let frame = Frame {
            pts: 7,
            width: 4,
            height: 4,
            format: PixelFormat::Rgba8,
            data: FrameData::Cpu {
                bytes: vec![200; 64],
                stride: 16,
            },
        };
        let t = thumbnail_of(&frame, 2).unwrap();
        assert_eq!((t.width, t.height, t.pts), (2, 2, 7));
        assert!(t.rgba.iter().all(|&b| b == 200));
    }
}
