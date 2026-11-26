//! Timeline audio mix at 48 kHz stereo f32 (D6): main-track clip audio at unity, music clips with
//! gain and linear fades, read in ranges from the disk audio cache. Shared by playback and
//! export, plus the export loudness normaliser (gain + true-peak limiter).

use crate::library::{AudioPcm, MediaEntry};
use editelier_core::{Id, Project, TICKS_PER_SECOND, Tick};
use editelier_export::{ExportError, LoudnessTarget, loudness};
use std::collections::HashMap;
use std::sync::Arc;

pub const RATE: i64 = 48_000;
const TICKS_PER_SAMPLE: i64 = TICKS_PER_SECOND / RATE;

pub fn sample_of(t: Tick) -> i64 {
    t.0.div_euclid(TICKS_PER_SAMPLE)
}

pub fn tick_of(sample: i64) -> Tick {
    Tick(sample * TICKS_PER_SAMPLE)
}

/// Add `src` (from source frame `src_at`) into `out` (stereo, timeline sample `out_at`) over
/// timeline samples `[from, to)`, scaled by `gain(k)`.
fn add(
    out: &mut [f32],
    out_at: i64,
    src: &AudioPcm,
    src_at: i64,
    (from, to): (i64, i64),
    gain: impl Fn(i64) -> f32,
) {
    let n = (out.len() / 2) as i64;
    let lo = from.max(out_at);
    let hi = to.min(out_at + n);
    if hi <= lo {
        return;
    }
    let mut buf = vec![0f32; (hi - lo) as usize * 2];
    src.read(src_at + (lo - from), &mut buf);
    for k in lo..hi {
        let (i, o) = ((k - lo) as usize * 2, (k - out_at) as usize * 2);
        let g = gain(k);
        out[o] += buf[i] * g;
        out[o + 1] += buf[i + 1] * g;
    }
}

/// Mix the timeline into `out` (stereo interleaved), whose first frame is timeline sample `at`.
/// `out` is overwritten.
pub fn mix(p: &Project, lib: &HashMap<Id, Arc<MediaEntry>>, at: i64, out: &mut [f32]) {
    out.fill(0.0);
    let end = at + (out.len() / 2) as i64;
    let audio = |m: Id| lib.get(&m).and_then(|e| e.audio.as_deref());
    for (start, c) in p.main_clips() {
        let (s0, s1) = (sample_of(start), sample_of(start + c.duration()));
        if s1 <= at || s0 >= end {
            continue;
        }
        if let Some(a) = audio(c.media) {
            add(out, at, a, sample_of(c.source_in), (s0, s1), |_| 1.0);
        }
    }
    for m in p.timeline.music.iter() {
        let (s0, s1) = (sample_of(m.start), sample_of(m.start + m.duration()));
        if s1 <= at || s0 >= end {
            continue;
        }
        let Some(a) = audio(m.media) else { continue };
        let gain = 10f32.powf(m.gain_db / 20.0);
        let (fi, fo) = (sample_of(m.fade_in), sample_of(m.fade_out));
        let env = |k: i64| {
            let (el, rem) = (k - s0, s1 - k);
            let up = if el < fi { el as f32 / fi as f32 } else { 1.0 };
            let down = if rem < fo {
                rem as f32 / fo as f32
            } else {
                1.0
            };
            gain * up.min(down)
        };
        add(out, at, a, sample_of(m.source_in), (s0, s1), env);
    }
}

/// Normalise a stereo mix to `target` (D6): measure, gain, and when the gain would push the true
/// peak over the ceiling, a lookahead true-peak limiter. Re-measures and corrects until loudness
/// is within 0.1 LU and the true peak is under the ceiling (the limiter costs a little loudness).
pub fn normalize(buf: &mut [f32], target: LoudnessTarget) -> Result<(), ExportError> {
    let r = loudness::measure(buf, target)?;
    if !r.integrated_lufs.is_finite() {
        return Ok(()); // silence
    }
    if !r.needs_limiter {
        let g = loudness::db_to_linear(r.gain_db);
        buf.iter_mut().for_each(|s| *s *= g);
        return Ok(());
    }
    // ponytail: keeps a copy of the mix for the retries (export already holds it in memory).
    let orig = buf.to_vec();
    let (mut gain, mut ceiling) = (r.gain_db, target.true_peak_dbtp - 0.2);
    for _ in 0..6 {
        let g = loudness::db_to_linear(gain);
        buf.iter_mut().zip(&orig).for_each(|(o, s)| *o = s * g);
        limit(buf, loudness::db_to_linear(ceiling));
        let m = loudness::measure(buf, target)?;
        let (lu, over) = (
            target.target_lufs - m.integrated_lufs,
            m.true_peak_dbtp - target.true_peak_dbtp,
        );
        if lu.abs() < 0.1 && over <= 0.0 {
            break;
        }
        gain += lu;
        if over > 0.0 {
            ceiling -= over + 0.05;
        }
    }
    Ok(())
}

/// Lookahead (2 ms) true-peak limiter on stereo interleaved audio, offline: 4x-oversampled peak
/// detection, gain = sliding minimum of the needed gain over the lookahead, box-smoothed over
/// the same window (so it is fully down when the peak arrives), 50 ms exponential release.
fn limit(buf: &mut [f32], ceiling: f32) {
    const LOOK: usize = 96;
    let frames = buf.len() / 2;
    if frames == 0 {
        return;
    }
    let taps = sinc_taps();
    // Needed gain per frame from the 4x-interpolated peak between this frame and the next.
    let need: Vec<f32> = (0..frames)
        .map(|i| {
            let p = (0..2)
                .map(|c| true_peak_at(buf, i, c, &taps))
                .fold(0f32, f32::max);
            if p > ceiling { ceiling / p } else { 1.0 }
        })
        .collect();
    // Forward sliding minimum over [i, i + LOOK) (monotonic deque).
    let mut mins = vec![1f32; frames];
    let mut q: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for j in (0..frames).rev() {
        while q.back().is_some_and(|&b| need[b] >= need[j]) {
            q.pop_back();
        }
        q.push_back(j);
        while q.front().is_some_and(|&f| f >= j + LOOK) {
            q.pop_front();
        }
        mins[j] = need[q[0]];
    }
    // Box average over (i - LOOK, i], then release.
    let rel = 1.0 - (-1.0f32 / (0.05 * RATE as f32)).exp();
    let (mut sum, mut env) = (0f64, 1f32);
    for i in 0..frames {
        sum += mins[i] as f64;
        if i >= LOOK {
            sum -= mins[i - LOOK] as f64;
        }
        // Before the start the window is padded with the first frame's gain.
        let pad = LOOK.saturating_sub(i + 1) as f64 * mins[0] as f64;
        let boxed = ((sum + pad) / LOOK as f64) as f32;
        env = boxed.min(env + (1.0 - env) * rel);
        buf[i * 2] *= env;
        buf[i * 2 + 1] *= env;
    }
}

const HALF: i64 = 8;

/// Hann-windowed sinc taps for the 1/4, 2/4 and 3/4 positions, for samples `i - 7 ..= i + 8`.
fn sinc_taps() -> [[f32; 16]; 3] {
    let mut t = [[0f32; 16]; 3];
    for (p, row) in t.iter_mut().enumerate() {
        let frac = (p + 1) as f32 / 4.0;
        for (n, w) in row.iter_mut().enumerate() {
            let d = std::f32::consts::PI * (frac - (n as i64 + 1 - HALF) as f32);
            *w = d.sin() / d * (0.5 + 0.5 * (d / HALF as f32).cos());
        }
    }
    t
}

/// Largest |x| of channel `c` over [i, i + 1] at 4x oversampling.
fn true_peak_at(buf: &[f32], i: usize, c: usize, taps: &[[f32; 16]; 3]) -> f32 {
    let frames = (buf.len() / 2) as i64;
    let x = |k: i64| {
        if (0..frames).contains(&k) {
            buf[k as usize * 2 + c]
        } else {
            0.0
        }
    };
    let i = i as i64;
    let mut peak = x(i).abs().max(x(i + 1).abs());
    for row in taps {
        let v: f32 = (0..16).map(|n| x(i + n + 1 - HALF) * row[n as usize]).sum();
        peak = peak.max(v.abs());
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{MediaEntry, Proxy};
    use editelier_core::{Command, MediaKind, MediaMeta, Session, Settings};

    /// An on-disk audio cache file holding `samples` (stereo interleaved).
    pub(crate) fn pcm(name: &str, samples: &[f32]) -> AudioPcm {
        let dir = std::env::temp_dir().join("editelier-mix-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.pcm"));
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        AudioPcm::open(&path).unwrap()
    }

    fn check_normalized(buf: &[f32]) {
        let m = loudness::measure(buf, loudness::SOCIAL).unwrap();
        eprintln!("normalized: {m:?}");
        assert!((m.integrated_lufs + 14.0).abs() <= 0.5, "{m:?}");
        assert!(m.true_peak_dbtp <= -1.0, "{m:?}");
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        // Decaying 3 kHz bursts at 4x the ceiling, the first one at sample 0.
        let mut buf: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let t = i as f32 / 48_000.0;
                let s =
                    2.0 * (-200.0 * (t % 0.5)).exp() * (t * 3000.0 * std::f32::consts::TAU).sin();
                [s, s]
            })
            .collect();
        limit(&mut buf, 0.5);
        let peak = buf.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak <= 0.5, "{peak}");
    }

    #[test]
    fn limiter_reaches_target_on_peaky_audio() {
        // Quiet bed plus full-scale clicks: plain gain to -14 LUFS would clip by several dB.
        let mut buf: Vec<f32> = (0..48_000 * 6)
            .flat_map(|i| {
                let t = i as f32 / 48_000.0;
                let click = (-200.0 * (t % 0.5)).exp() * (t * 3000.0 * std::f32::consts::TAU).sin();
                let s = 0.05 * (t * 220.0 * std::f32::consts::TAU).sin() + 0.95 * click;
                [s, s]
            })
            .collect();
        let before = loudness::measure(&buf, loudness::SOCIAL).unwrap();
        assert!(before.needs_limiter, "{before:?}");
        normalize(&mut buf, loudness::SOCIAL).unwrap();
        check_normalized(&buf);
    }

    #[test]
    fn loud_peaky_fixture_normalizes() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test-media/loud_peaky_44k.wav");
        let Ok(tools) = editelier_media::Tools::locate() else {
            return eprintln!("SKIP: no FFmpeg helper");
        };
        if !p.is_file() {
            return eprintln!(
                "SKIP: {} missing; run scripts/make-test-media.ps1",
                p.display()
            );
        }
        let info = editelier_media::probe(&tools, &p).unwrap();
        let a = editelier_media::win::decode_audio(&tools, &p, &info.audio[0]).unwrap();
        assert_eq!((a.sample_rate, a.channels), (48_000, 2));
        let mut buf = a.samples;
        assert!(
            loudness::measure(&buf, loudness::SOCIAL)
                .unwrap()
                .needs_limiter
        );
        normalize(&mut buf, loudness::SOCIAL).unwrap();
        check_normalized(&buf);
    }

    #[test]
    fn music_gain_and_fades() {
        let mut s = Session::new(Project::new(Settings::default()));
        let sec = Tick(TICKS_PER_SECOND);
        let id = s
            .execute(&Command::AddMedia {
                path: "m.wav".into(),
                meta: MediaMeta {
                    kind: MediaKind::Audio,
                    duration: Tick(sec.0 * 4),
                    fps: None,
                    is_vfr: false,
                    has_audio: true,
                },
            })
            .unwrap()
            .unwrap();
        let m = s
            .execute(&Command::AddMusic {
                media: id,
                start: sec,
                source_in: Tick::ZERO,
                source_out: Tick(sec.0 * 2),
            })
            .unwrap()
            .unwrap();
        s.execute(&Command::SetMusicGain {
            id: m,
            gain_db: -6.0206,
            fade_in: sec,
            fade_out: Tick::ZERO,
        })
        .unwrap();
        let entry = MediaEntry {
            path: "m.wav".into(),
            probe: editelier_media::ProbeInfo {
                path: "m.wav".into(),
                kind: editelier_media::MediaKind::Audio,
                container: String::new(),
                duration_us: None,
                video: vec![],
                audio: vec![],
                image: None,
            },
            index: None,
            color: editelier_render::SourceColor::from_tags(None, None, 1080),
            thumb: None,
            proxy: Proxy::None,
            audio: Some(Arc::new(pcm("gain", &vec![1.0; 48_000 * 4 * 2]))),
        };
        let lib = HashMap::from([(id, Arc::new(entry))]);
        let mut out = vec![0.0; 48_000 * 4 * 2];
        mix(s.project(), &lib, 0, &mut out);
        assert_eq!(out[0], 0.0); // before the clip
        let at = |sample: usize| out[sample * 2];
        assert!((at(48_000 + 24_000) - 0.25).abs() < 1e-3); // half way up the fade, -6 dB
        assert!((at(48_000 * 2 + 10) - 0.5).abs() < 1e-3);
        assert_eq!(at(48_000 * 3 + 1), 0.0); // after the clip
    }
}
