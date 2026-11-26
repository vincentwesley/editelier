//! Every edit is a named, serde-serialisable [`Command`]: the UI, CLI and agent channel all send
//! the same JSON (`{"cmd":"trim_end","id":3,"source_out":...}`), and [`crate::Session`] makes each
//! one undoable. All times are frame-snapped to the project rate so cuts land on frame boundaries.

use crate::Error;
use crate::model::{
    Clip, Id, MediaKind, MediaMeta, MediaRef, MusicClip, Project, TitleClip, TitleStyle,
};
use craft_time::Tick;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Add a probed file to the media pool. Returns the media id.
    AddMedia {
        path: String,
        meta: MediaMeta,
    },
    /// Insert a main-track clip before `index` (append when `None`). Returns the clip id.
    InsertClip {
        media: Id,
        source_in: Tick,
        source_out: Tick,
        index: Option<usize>,
    },
    /// Move a main-track clip's in point; downstream clips ripple. `drag` coalesces one gesture.
    TrimStart {
        id: Id,
        source_in: Tick,
        drag: Option<u64>,
    },
    /// Move a main-track clip's out point; downstream clips ripple. `drag` coalesces one gesture.
    TrimEnd {
        id: Id,
        source_out: Tick,
        drag: Option<u64>,
    },
    /// Cut the main-track clip under the playhead at the start of the frame containing `at`.
    /// Returns the id of the right half.
    Split {
        at: Tick,
    },
    /// Merge main-track clip `id` with the next one (same media, contiguous source).
    Join {
        id: Id,
    },
    /// Remove a main-track clip; the magnetic track closes the gap.
    RippleDelete {
        id: Id,
    },
    /// Reorder a main-track clip to `index`.
    MoveClip {
        id: Id,
        index: usize,
    },
    /// Returns the title id. `style` and `position` default to the house style, centred.
    AddTitle {
        start: Tick,
        duration: Tick,
        text: String,
        style: Option<TitleStyle>,
        position: Option<[f32; 2]>,
    },
    EditTitle {
        id: Id,
        start: Option<Tick>,
        duration: Option<Tick>,
        text: Option<String>,
        style: Option<TitleStyle>,
        position: Option<[f32; 2]>,
    },
    DeleteTitle {
        id: Id,
    },
    /// Returns the music clip id. Starts at 0 dB with no fades.
    AddMusic {
        media: Id,
        start: Tick,
        source_in: Tick,
        source_out: Tick,
    },
    EditMusic {
        id: Id,
        start: Option<Tick>,
        source_in: Option<Tick>,
        source_out: Option<Tick>,
    },
    SetMusicGain {
        id: Id,
        gain_db: f32,
        fade_in: Tick,
        fade_out: Tick,
    },
    DeleteMusic {
        id: Id,
    },
    /// Rename the project (1..=120 characters after trimming).
    Rename {
        name: String,
    },
    /// Change the canvas size; `aspect` is the shell's aspect id, kept for display.
    SetCanvas {
        aspect: String,
        width: u32,
        height: u32,
    },
    /// Cut title `id` at the frame containing `at`. Returns the id of the right half.
    SplitTitle {
        id: Id,
        at: Tick,
    },
    /// Cut music clip `id` at the frame containing `at`; the halves keep the outer fades.
    /// Returns the id of the right half.
    SplitMusic {
        id: Id,
        at: Tick,
    },
}

/// Gain range a music clip accepts, in dB.
pub const GAIN_DB_RANGE: std::ops::RangeInclusive<f32> = -96.0..=24.0;

impl Command {
    /// The registry id, identical to the JSON `cmd` tag; also the undo label.
    pub fn name(&self) -> &'static str {
        match self {
            Command::AddMedia { .. } => "add_media",
            Command::InsertClip { .. } => "insert_clip",
            Command::TrimStart { .. } => "trim_start",
            Command::TrimEnd { .. } => "trim_end",
            Command::Split { .. } => "split",
            Command::Join { .. } => "join",
            Command::RippleDelete { .. } => "ripple_delete",
            Command::MoveClip { .. } => "move_clip",
            Command::AddTitle { .. } => "add_title",
            Command::EditTitle { .. } => "edit_title",
            Command::DeleteTitle { .. } => "delete_title",
            Command::AddMusic { .. } => "add_music",
            Command::EditMusic { .. } => "edit_music",
            Command::SetMusicGain { .. } => "set_music_gain",
            Command::DeleteMusic { .. } => "delete_music",
            Command::Rename { .. } => "rename",
            Command::SetCanvas { .. } => "set_canvas",
            Command::SplitTitle { .. } => "split_title",
            Command::SplitMusic { .. } => "split_music",
        }
    }

    /// Drag gesture id, for coalescing trims into one undo step.
    pub fn drag(&self) -> Option<u64> {
        match self {
            Command::TrimStart { drag, .. } | Command::TrimEnd { drag, .. } => *drag,
            _ => None,
        }
    }

    /// Apply to `p` in place. On error `p` may be partly changed; [`crate::Session`] discards it.
    /// Returns the id of anything created.
    pub fn apply(&self, p: &mut Project) -> Result<Option<Id>, Error> {
        let fps = p.settings.fps;
        let frame = fps.frame_duration();
        let snap = |t: Tick| fps.snap_nearest(t);
        match self {
            Command::AddMedia { path, meta } => {
                if meta.duration < Tick::ZERO {
                    return Err(Error::Invalid("media duration is negative"));
                }
                let id = p.alloc_id();
                Arc::make_mut(&mut p.media).push(MediaRef {
                    id,
                    path: path.clone(),
                    meta: meta.clone(),
                });
                Ok(Some(id))
            }
            Command::InsertClip {
                media,
                source_in,
                source_out,
                index,
            } => {
                let m = media_ref(p, *media)?;
                if m.meta.kind == MediaKind::Audio {
                    return Err(Error::Invalid(
                        "audio-only media can't go on the video track",
                    ));
                }
                let (source_in, source_out) = source_range(*source_in, *source_out, m, p)?;
                let main = &p.timeline.main;
                let index = index.unwrap_or(main.len());
                if index > main.len() {
                    return Err(Error::Invalid("insert index is past the end of the track"));
                }
                let id = p.alloc_id();
                Arc::make_mut(&mut p.timeline.main).insert(
                    index,
                    Clip {
                        id,
                        media: *media,
                        source_in,
                        source_out,
                    },
                );
                Ok(Some(id))
            }
            Command::TrimStart { id, source_in, .. } => {
                let i = main_index(p, *id)?;
                let c = &mut Arc::make_mut(&mut p.timeline.main)[i];
                c.source_in = snap(*source_in).clamp(Tick::ZERO, c.source_out - frame);
                Ok(None)
            }
            Command::TrimEnd { id, source_out, .. } => {
                let i = main_index(p, *id)?;
                let end = source_end(media_ref(p, p.timeline.main[i].media)?, p);
                let c = &mut Arc::make_mut(&mut p.timeline.main)[i];
                c.source_out =
                    snap(*source_out).clamp(c.source_in + frame, end.max(c.source_in + frame));
                Ok(None)
            }
            Command::Split { at } => {
                let at = fps.snap(*at);
                let (i, start, _) = p
                    .clip_at(at)
                    .ok_or(Error::Invalid("no clip under the playhead"))?;
                if at == start {
                    return Err(Error::Invalid("the playhead is already on a cut"));
                }
                let id = p.alloc_id();
                let main = Arc::make_mut(&mut p.timeline.main);
                let cut = main[i].source_in + (at - start);
                let right = Clip {
                    id,
                    source_in: cut,
                    ..main[i].clone()
                };
                main[i].source_out = cut;
                main.insert(i + 1, right);
                Ok(Some(id))
            }
            Command::Join { id } => {
                let i = main_index(p, *id)?;
                let main = &p.timeline.main;
                match main.get(i + 1) {
                    Some(r) if r.media == main[i].media && r.source_in == main[i].source_out => {
                        let main = Arc::make_mut(&mut p.timeline.main);
                        let r = main.remove(i + 1);
                        main[i].source_out = r.source_out;
                        Ok(None)
                    }
                    _ => Err(Error::Invalid(
                        "only a clip followed by its own continuation can be joined",
                    )),
                }
            }
            Command::RippleDelete { id } => {
                let i = main_index(p, *id)?;
                Arc::make_mut(&mut p.timeline.main).remove(i);
                Ok(None)
            }
            Command::MoveClip { id, index } => {
                let i = main_index(p, *id)?;
                if *index >= p.timeline.main.len() {
                    return Err(Error::Invalid("move index is past the end of the track"));
                }
                let main = Arc::make_mut(&mut p.timeline.main);
                let c = main.remove(i);
                main.insert(*index, c);
                Ok(None)
            }
            Command::AddTitle {
                start,
                duration,
                text,
                style,
                position,
            } => {
                let mut t = TitleClip {
                    id: 0,
                    start: Tick::ZERO,
                    duration: frame,
                    text: text.clone(),
                    style: style.clone().unwrap_or_default(),
                    position: position.unwrap_or([0.5, 0.5]),
                };
                set_title_times(&mut t, *start, *duration, p)?;
                check_title_look(&t)?;
                t.id = p.alloc_id();
                let id = t.id;
                Arc::make_mut(&mut p.timeline.titles).push(t);
                Ok(Some(id))
            }
            Command::EditTitle {
                id,
                start,
                duration,
                text,
                style,
                position,
            } => {
                let i = p
                    .timeline
                    .titles
                    .iter()
                    .position(|t| t.id == *id)
                    .ok_or(Error::NotFound(*id))?;
                let mut t = p.timeline.titles[i].clone();
                let (s, d) = (start.unwrap_or(t.start), duration.unwrap_or(t.duration));
                set_title_times(&mut t, s, d, p)?;
                if let Some(x) = text {
                    t.text = x.clone();
                }
                if let Some(x) = style {
                    t.style = x.clone();
                }
                if let Some(x) = position {
                    t.position = *x;
                }
                check_title_look(&t)?;
                Arc::make_mut(&mut p.timeline.titles)[i] = t;
                Ok(None)
            }
            Command::DeleteTitle { id } => {
                let i = p
                    .timeline
                    .titles
                    .iter()
                    .position(|t| t.id == *id)
                    .ok_or(Error::NotFound(*id))?;
                Arc::make_mut(&mut p.timeline.titles).remove(i);
                Ok(None)
            }
            Command::AddMusic {
                media,
                start,
                source_in,
                source_out,
            } => {
                let m = music_media(p, *media)?;
                let (source_in, source_out) = source_range(*source_in, *source_out, m, p)?;
                let id = p.alloc_id();
                let clip = MusicClip {
                    id,
                    media: *media,
                    start: snap(*start).max(Tick::ZERO),
                    source_in,
                    source_out,
                    gain_db: 0.0,
                    fade_in: Tick::ZERO,
                    fade_out: Tick::ZERO,
                };
                Arc::make_mut(&mut p.timeline.music).push(clip);
                Ok(Some(id))
            }
            Command::EditMusic {
                id,
                start,
                source_in,
                source_out,
            } => {
                let i = music_index(p, *id)?;
                let mut c = p.timeline.music[i].clone();
                let m = music_media(p, c.media)?;
                (c.source_in, c.source_out) = source_range(
                    source_in.unwrap_or(c.source_in),
                    source_out.unwrap_or(c.source_out),
                    m,
                    p,
                )?;
                c.start = snap(start.unwrap_or(c.start)).max(Tick::ZERO);
                // Keep the fades inside the (maybe shorter) clip.
                c.fade_in = c.fade_in.min(c.duration());
                c.fade_out = c.fade_out.min(c.duration() - c.fade_in);
                Arc::make_mut(&mut p.timeline.music)[i] = c;
                Ok(None)
            }
            Command::SetMusicGain {
                id,
                gain_db,
                fade_in,
                fade_out,
            } => {
                let i = music_index(p, *id)?;
                if !GAIN_DB_RANGE.contains(gain_db) {
                    return Err(Error::Invalid("gain must be between -96 dB and +24 dB"));
                }
                let c = &p.timeline.music[i];
                if *fade_in < Tick::ZERO
                    || *fade_out < Tick::ZERO
                    || *fade_in + *fade_out > c.duration()
                {
                    return Err(Error::Invalid(
                        "fades must be non-negative and fit inside the clip",
                    ));
                }
                let c = &mut Arc::make_mut(&mut p.timeline.music)[i];
                (c.gain_db, c.fade_in, c.fade_out) = (*gain_db, *fade_in, *fade_out);
                Ok(None)
            }
            Command::DeleteMusic { id } => {
                let i = music_index(p, *id)?;
                Arc::make_mut(&mut p.timeline.music).remove(i);
                Ok(None)
            }
            Command::Rename { name } => {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > 120 {
                    return Err(Error::Invalid("a project name must be 1 to 120 characters"));
                }
                p.info.name = name.to_owned();
                Ok(None)
            }
            Command::SetCanvas {
                aspect,
                width,
                height,
            } => {
                let ok = |v: &u32| (2..=8192).contains(v) && v.is_multiple_of(2);
                if !ok(width) || !ok(height) || aspect.len() > 16 {
                    return Err(Error::Invalid(
                        "canvas sides must be even and between 2 and 8192 pixels",
                    ));
                }
                (p.settings.width, p.settings.height) = (*width, *height);
                p.info.aspect = aspect.clone();
                Ok(None)
            }
            Command::SplitTitle { id, at } => {
                let i = p
                    .timeline
                    .titles
                    .iter()
                    .position(|t| t.id == *id)
                    .ok_or(Error::NotFound(*id))?;
                let at = fps.snap(*at);
                let t = &p.timeline.titles[i];
                if at <= t.start || at >= t.start + t.duration {
                    return Err(Error::Invalid("the playhead is not inside that title"));
                }
                let mut right = TitleClip {
                    start: at,
                    duration: t.start + t.duration - at,
                    ..t.clone()
                };
                right.id = p.alloc_id();
                let rid = right.id;
                let titles = Arc::make_mut(&mut p.timeline.titles);
                titles[i].duration = at - titles[i].start;
                titles.insert(i + 1, right);
                Ok(Some(rid))
            }
            Command::SplitMusic { id, at } => {
                let i = music_index(p, *id)?;
                let at = fps.snap(*at);
                let c = &p.timeline.music[i];
                if at <= c.start || at >= c.start + c.duration() {
                    return Err(Error::Invalid("the playhead is not inside that music clip"));
                }
                let cut = c.source_in + (at - c.start);
                let mut right = MusicClip {
                    start: at,
                    source_in: cut,
                    fade_in: Tick::ZERO,
                    fade_out: c.fade_out.min(c.source_out - cut),
                    ..c.clone()
                };
                right.id = p.alloc_id();
                let rid = right.id;
                let music = Arc::make_mut(&mut p.timeline.music);
                let left = &mut music[i];
                left.source_out = cut;
                left.fade_out = Tick::ZERO;
                left.fade_in = left.fade_in.min(left.duration());
                music.insert(i + 1, right);
                Ok(Some(rid))
            }
        }
    }
}

fn media_ref(p: &Project, id: Id) -> Result<&MediaRef, Error> {
    p.media(id).ok_or(Error::NotFound(id))
}

fn music_media(p: &Project, id: Id) -> Result<&MediaRef, Error> {
    let m = media_ref(p, id)?;
    if m.meta.kind == MediaKind::Image || !m.meta.has_audio {
        return Err(Error::Invalid("this media has no audio"));
    }
    Ok(m)
}

fn main_index(p: &Project, id: Id) -> Result<usize, Error> {
    p.timeline
        .main
        .iter()
        .position(|c| c.id == id)
        .ok_or(Error::NotFound(id))
}

fn music_index(p: &Project, id: Id) -> Result<usize, Error> {
    p.timeline
        .music
        .iter()
        .position(|c| c.id == id)
        .ok_or(Error::NotFound(id))
}

/// Last frame-grid point inside the media.
fn source_end(m: &MediaRef, p: &Project) -> Tick {
    let end = m.end();
    if end == Tick::MAX {
        end
    } else {
        p.settings.fps.snap(end)
    }
}

/// Frame-snap a source range and clamp it to the media; at least one frame long.
fn source_range(
    source_in: Tick,
    source_out: Tick,
    m: &MediaRef,
    p: &Project,
) -> Result<(Tick, Tick), Error> {
    let fps = p.settings.fps;
    let source_in = fps.snap_nearest(source_in).max(Tick::ZERO);
    let source_out = fps.snap_nearest(source_out).min(source_end(m, p));
    if source_out - source_in < fps.frame_duration() {
        return Err(Error::Invalid(
            "a clip must be at least one frame long and inside its media",
        ));
    }
    Ok((source_in, source_out))
}

fn set_title_times(
    t: &mut TitleClip,
    start: Tick,
    duration: Tick,
    p: &Project,
) -> Result<(), Error> {
    let fps = p.settings.fps;
    t.start = fps.snap_nearest(start).max(Tick::ZERO);
    t.duration = fps.snap_nearest(duration);
    if t.duration < fps.frame_duration() {
        return Err(Error::Invalid("a title must be at least one frame long"));
    }
    Ok(())
}

fn check_title_look(t: &TitleClip) -> Result<(), Error> {
    let size_ok = t.style.size.is_finite() && t.style.size > 0.0;
    if !size_ok || !t.position.iter().all(|v| v.is_finite()) {
        return Err(Error::Invalid(
            "title size and position must be finite numbers",
        ));
    }
    Ok(())
}
