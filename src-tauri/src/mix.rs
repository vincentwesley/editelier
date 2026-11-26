//! Timeline audio mix at 48 kHz stereo f32 (D6): main-track clip audio at unity, music clips with
//! gain and linear fades. Shared by playback and export.
//! ponytail: whole files are decoded into memory (48 kHz stereo f32 ≈ 23 MB per minute); stream
//! from disk when projects get long.

use crate::library::MediaEntry;
use editelier_core::{Id, Project, TICKS_PER_SECOND, Tick};
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

/// Add `src` (stereo interleaved, starting at source sample `src_at`) into `out` (stereo,
/// timeline sample `out_at`) over timeline samples `[from, to)`, scaled by `gain(k)`.
fn add(
    out: &mut [f32],
    out_at: i64,
    src: &[f32],
    src_at: i64,
    (from, to): (i64, i64),
    gain: impl Fn(i64) -> f32,
) {
    let n = (out.len() / 2) as i64;
    let lo = from.max(out_at);
    let hi = to.min(out_at + n);
    for k in lo..hi {
        let s = (src_at + (k - from)) as usize * 2;
        if s + 1 >= src.len() {
            break;
        }
        let g = gain(k);
        let o = (k - out_at) as usize * 2;
        out[o] += src[s] * g;
        out[o + 1] += src[s + 1] * g;
    }
}

/// Mix the timeline into `out` (stereo interleaved), whose first frame is timeline sample `at`.
/// `out` is overwritten.
pub fn mix(p: &Project, lib: &HashMap<Id, Arc<MediaEntry>>, at: i64, out: &mut [f32]) {
    out.fill(0.0);
    let end = at + (out.len() / 2) as i64;
    let audio = |m: Id| lib.get(&m).and_then(|e| e.audio.clone());
    for (start, c) in p.main_clips() {
        let (s0, s1) = (sample_of(start), sample_of(start + c.duration()));
        if s1 <= at || s0 >= end {
            continue;
        }
        if let Some(a) = audio(c.media) {
            add(out, at, &a, sample_of(c.source_in), (s0, s1), |_| 1.0);
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
        add(out, at, &a, sample_of(m.source_in), (s0, s1), env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{MediaEntry, Proxy};
    use editelier_core::{Command, MediaKind, MediaMeta, Session, Settings};

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
            audio: Some(Arc::new(vec![1.0; 48_000 * 4 * 2])),
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
