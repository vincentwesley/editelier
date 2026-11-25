//! Project model and read-only queries. Collections are `Arc`'d so a project clone is cheap and an
//! edit copies only the collection it touches (copy-on-write undo snapshots).

use craft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Stable id for media, clips, titles and music clips; unique within a project, never reused.
pub type Id = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub sample_rate: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_30,
            sample_rate: 48_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

/// What the media layer's probe found. `fps` is the nominal rate (`None` for audio and stills).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaMeta {
    pub kind: MediaKind,
    pub duration: Tick,
    pub fps: Option<FrameRate>,
    pub is_vfr: bool,
    pub has_audio: bool,
}

/// A media pool entry. `path` is stored as given; the app layer validates it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: Id,
    pub path: String,
    pub meta: MediaMeta,
}

impl MediaRef {
    /// Latest usable source time: stills have no end, everything else stops at its duration.
    pub fn end(&self) -> Tick {
        if self.meta.kind == MediaKind::Image {
            Tick::MAX
        } else {
            self.meta.duration
        }
    }
}

/// A clip on the magnetic main track. Its timeline start is the sum of the clips before it.
/// Speed is fixed at 1.0 in M1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub media: Id,
    pub source_in: Tick,
    pub source_out: Tick,
}

impl Clip {
    pub fn duration(&self) -> Tick {
        self.source_out - self.source_in
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TitleStyle {
    pub font: String,
    /// Font size in canvas pixels.
    pub size: f32,
    /// Straight RGBA.
    pub color: [u8; 4],
}

impl Default for TitleStyle {
    fn default() -> Self {
        TitleStyle {
            font: "Space Grotesk".into(),
            size: 96.0,
            color: [255, 255, 255, 255],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TitleClip {
    pub id: Id,
    pub start: Tick,
    pub duration: Tick,
    pub text: String,
    pub style: TitleStyle,
    /// Anchor centre in canvas units, 0..1 on each axis.
    pub position: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MusicClip {
    pub id: Id,
    pub media: Id,
    pub start: Tick,
    pub source_in: Tick,
    pub source_out: Tick,
    pub gain_db: f32,
    pub fade_in: Tick,
    pub fade_out: Tick,
}

impl MusicClip {
    pub fn duration(&self) -> Tick {
        self.source_out - self.source_in
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub main: Arc<Vec<Clip>>,
    pub titles: Arc<Vec<TitleClip>>,
    pub music: Arc<Vec<MusicClip>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub settings: Settings,
    pub media: Arc<Vec<MediaRef>>,
    pub timeline: Timeline,
    /// Next id to hand out.
    pub next_id: Id,
}

/// One thing to draw or mix at a timeline time, bottom to top: main video, then titles, then music.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer<'a> {
    /// `source_time` is on the source's frame grid for CFR media; for VFR it's the raw source time
    /// and the media layer shows the latest frame with PTS ≤ it (D4).
    Video {
        clip: Id,
        media: Id,
        source_time: Tick,
    },
    Title(&'a TitleClip),
    /// `fade` is the 0..1 envelope from the fades, applied on top of `gain_db`.
    Music {
        clip: Id,
        media: Id,
        source_time: Tick,
        gain_db: f32,
        fade: f32,
    },
}

impl Project {
    pub fn new(settings: Settings) -> Self {
        Project {
            settings,
            next_id: 1,
            ..Default::default()
        }
    }

    pub(crate) fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: Id) -> Option<&MediaRef> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Main-track clips with their timeline start.
    pub fn main_clips(&self) -> impl Iterator<Item = (Tick, &Clip)> {
        self.timeline.main.iter().scan(Tick::ZERO, |start, c| {
            let s = *start;
            *start = s + c.duration();
            Some((s, c))
        })
    }

    /// The main-track clip under `t` (half-open), with its index and timeline start.
    pub fn clip_at(&self, t: Tick) -> Option<(usize, Tick, &Clip)> {
        self.main_clips()
            .enumerate()
            .find(|(_, (s, c))| t >= *s && t < *s + c.duration())
            .map(|(i, (s, c))| (i, s, c))
    }

    /// End of the last thing on any track.
    pub fn duration(&self) -> Tick {
        let main = self
            .timeline
            .main
            .iter()
            .fold(Tick::ZERO, |acc, c| acc + c.duration());
        let titles = self.timeline.titles.iter().map(|t| t.start + t.duration);
        let music = self.timeline.music.iter().map(|m| m.start + m.duration());
        titles.chain(music).fold(main, Tick::max)
    }

    /// What to render at `t`, which is first snapped down to the timeline frame containing it.
    pub fn layers_at(&self, t: Tick) -> Vec<Layer<'_>> {
        let t = self.settings.fps.snap(t);
        let mut out = Vec::new();
        if let Some((_, start, c)) = self.clip_at(t) {
            let raw = c.source_in + (t - start);
            let source_time = match self.media(c.media).map(|m| &m.meta) {
                Some(MediaMeta {
                    kind: MediaKind::Video,
                    fps: Some(fps),
                    is_vfr: false,
                    ..
                }) => fps.snap(raw),
                _ => raw,
            };
            out.push(Layer::Video {
                clip: c.id,
                media: c.media,
                source_time,
            });
        }
        out.extend(
            self.timeline
                .titles
                .iter()
                .filter(|x| t >= x.start && t < x.start + x.duration)
                .map(Layer::Title),
        );
        for m in self
            .timeline
            .music
            .iter()
            .filter(|m| t >= m.start && t < m.start + m.duration())
        {
            let elapsed = t - m.start;
            let remaining = m.duration() - elapsed;
            let ramp = |part: Tick, len: Tick| {
                if len > Tick::ZERO && part < len {
                    (part.0 as f64 / len.0 as f64) as f32
                } else {
                    1.0
                }
            };
            out.push(Layer::Music {
                clip: m.id,
                media: m.media,
                source_time: m.source_in + elapsed,
                gain_db: m.gain_db,
                fade: ramp(elapsed, m.fade_in).min(ramp(remaining, m.fade_out)),
            });
        }
        out
    }
}
