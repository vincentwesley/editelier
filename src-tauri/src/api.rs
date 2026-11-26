//! The UI contract (`ui/src/api/types.ts`): snapshot shapes, error codes, and the translation of
//! the UI's named edits into core commands. Ids travel as strings; core ids are numbers.

use crate::library::{Library, Proxy};
use editelier_core::{
    Command, Error as CoreError, FrameRate, Id, MediaKind, Project, Session, TICKS_PER_SECOND,
    Tick, TitleClip, TitleStyle,
};
use serde::Serialize;
use serde_json::Value;

/// Error shape the UI expects from every command: `code` picks the friendly message.
#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        ApiError {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        let code = match e {
            CoreError::NotFound(_) => "not_found",
            CoreError::Invalid(_) => "invalid",
            _ => "invalid_project",
        };
        ApiError::new(code, e.to_string())
    }
}

impl From<editelier_media::Error> for ApiError {
    fn from(e: editelier_media::Error) -> Self {
        let msg = e.to_string();
        let code = match &e {
            editelier_media::Error::Unsupported(m) if m.contains("HEVC") => {
                "hevc_extension_missing"
            }
            editelier_media::Error::Unsupported(_) => "unsupported_codec",
            editelier_media::Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                "media_offline"
            }
            editelier_media::Error::Cancelled => "cancelled",
            _ => "media_error",
        };
        ApiError::new(code, msg)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn invalid(msg: &str) -> ApiError {
    ApiError::new("invalid", msg)
}

// ---- snapshot ----

#[derive(Serialize)]
pub struct Rational {
    num: i64,
    den: i64,
}

impl From<FrameRate> for Rational {
    fn from(f: FrameRate) -> Self {
        Rational {
            num: f.num,
            den: f.den,
        }
    }
}

#[derive(Serialize)]
struct Canvas {
    aspect: String,
    width: u32,
    height: u32,
    frame_rate: Rational,
}

#[derive(Serialize)]
struct Probe {
    duration: i64,
    width: u32,
    height: u32,
    frame_rate: Rational,
    is_vfr: bool,
    has_video: bool,
    has_audio: bool,
    codec: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProxyState {
    None,
    Building { progress: f32 },
    Ready,
    Failed { error: String },
}

impl From<&Proxy> for ProxyState {
    fn from(p: &Proxy) -> Self {
        match p {
            Proxy::None => ProxyState::None,
            Proxy::Building(f) => ProxyState::Building {
                progress: (f * 100.0).round(),
            },
            Proxy::Ready { .. } => ProxyState::Ready,
            Proxy::Failed(e) => ProxyState::Failed { error: e.clone() },
        }
    }
}

#[derive(Serialize)]
struct MediaOut {
    id: String,
    path: String,
    name: String,
    kind: &'static str,
    probe: Probe,
    proxy: ProxyState,
    thumbnail: Option<String>,
}

#[derive(Serialize)]
struct ClipOut {
    id: String,
    media_id: String,
    source_in: i64,
    source_out: i64,
}

#[derive(Serialize)]
struct StyleOut {
    font: String,
    size: f32,
    color: String,
    position: &'static str,
}

#[derive(Serialize)]
struct TitleOut {
    id: String,
    start: i64,
    duration: i64,
    text: String,
    style: StyleOut,
}

#[derive(Serialize)]
struct MusicOut {
    id: String,
    media_id: String,
    start: i64,
    source_in: i64,
    source_out: i64,
    gain_db: f32,
    fade_in: i64,
    fade_out: i64,
}

#[derive(Serialize)]
struct TimelineOut {
    main: Vec<ClipOut>,
    titles: Vec<TitleOut>,
    music: Vec<MusicOut>,
}

#[derive(Serialize)]
struct ProjectOut {
    id: String,
    name: String,
    path: Option<String>,
    canvas: Canvas,
    export_preset: String,
    keymap: String,
    media: Vec<MediaOut>,
    timeline: TimelineOut,
}

#[derive(Serialize)]
pub struct Snapshot {
    project: ProjectOut,
    undo_label: Option<&'static str>,
    redo_label: Option<&'static str>,
}

/// Friendly undo labels for core command names.
pub fn label(name: &'static str) -> &'static str {
    match name {
        "add_media" => "Import media",
        "insert_clip" => "Add clip",
        "trim_start" | "trim_end" => "Trim",
        "split" | "split_title" | "split_music" => "Split",
        "join" => "Join clips",
        "ripple_delete" | "delete_title" | "delete_music" => "Delete",
        "move_clip" => "Move clip",
        "add_title" => "Add title",
        "edit_title" => "Edit title",
        "add_music" => "Add music",
        "edit_music" => "Move music",
        "set_music_gain" => "Music volume and fades",
        "rename" => "Rename project",
        "set_canvas" => "Change canvas",
        other => other,
    }
}

const POSITIONS: [(&str, f32); 3] = [("top", 0.15), ("center", 0.5), ("lower-third", 0.82)];

fn position_name(p: [f32; 2]) -> &'static str {
    POSITIONS
        .iter()
        .min_by(|a, b| (a.1 - p[1]).abs().total_cmp(&(b.1 - p[1]).abs()))
        .map_or("center", |x| x.0)
}

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?, 255])
}

pub fn snapshot(
    session: &Session,
    path: Option<&std::path::Path>,
    doc_id: u64,
    lib: &Library,
) -> Snapshot {
    let p = session.project();
    let lib = lib.read().unwrap();
    let media = p
        .media
        .iter()
        .map(|m| {
            let e = lib.get(&m.id);
            let v = e.and_then(|e| e.probe.video.first());
            let img = e.and_then(|e| e.probe.image.as_ref());
            let (width, height) = v
                .map(|v| (v.width, v.height))
                .or(img.map(|i| (i.width, i.height)))
                .unwrap_or((0, 0));
            let codec = v
                .map(|v| v.codec.clone())
                .or(img.map(|i| i.format.clone()))
                .or(e.and_then(|e| e.probe.audio.first().map(|a| a.codec.clone())))
                .unwrap_or_default();
            let name = std::path::Path::new(&m.path)
                .file_name()
                .map_or(m.path.clone(), |n| n.to_string_lossy().into_owned());
            MediaOut {
                id: m.id.to_string(),
                path: m.path.clone(),
                name,
                kind: match m.meta.kind {
                    MediaKind::Video => "video",
                    MediaKind::Audio => "audio",
                    MediaKind::Image => "image",
                },
                probe: Probe {
                    duration: m.meta.duration.0,
                    width,
                    height,
                    frame_rate: m.meta.fps.unwrap_or(p.settings.fps).into(),
                    is_vfr: m.meta.is_vfr,
                    has_video: m.meta.kind != MediaKind::Audio,
                    has_audio: m.meta.has_audio,
                    codec,
                },
                proxy: e.map_or(ProxyState::None, |e| (&e.proxy).into()),
                thumbnail: e.and_then(|e| e.thumb.as_deref()).map(asset_url),
            }
        })
        .collect();
    let t = &p.timeline;
    Snapshot {
        project: ProjectOut {
            id: doc_id.to_string(),
            name: p.info.name.clone(),
            path: path.map(|p| p.display().to_string()),
            canvas: Canvas {
                aspect: p.info.aspect.clone(),
                width: p.settings.width,
                height: p.settings.height,
                frame_rate: p.settings.fps.into(),
            },
            export_preset: p.info.export_preset.clone(),
            keymap: p.info.keymap.clone(),
            media,
            timeline: TimelineOut {
                main: t
                    .main
                    .iter()
                    .map(|c| ClipOut {
                        id: c.id.to_string(),
                        media_id: c.media.to_string(),
                        source_in: c.source_in.0,
                        source_out: c.source_out.0,
                    })
                    .collect(),
                titles: t
                    .titles
                    .iter()
                    .map(|c| TitleOut {
                        id: c.id.to_string(),
                        start: c.start.0,
                        duration: c.duration.0,
                        text: c.text.clone(),
                        style: StyleOut {
                            font: c.style.font.clone(),
                            size: c.style.size,
                            color: hex(c.style.color),
                            position: position_name(c.position),
                        },
                    })
                    .collect(),
                music: t
                    .music
                    .iter()
                    .map(|c| MusicOut {
                        id: c.id.to_string(),
                        media_id: c.media.to_string(),
                        start: c.start.0,
                        source_in: c.source_in.0,
                        source_out: c.source_out.0,
                        gain_db: c.gain_db,
                        fade_in: c.fade_in.0,
                        fade_out: c.fade_out.0,
                    })
                    .collect(),
            },
        },
        undo_label: session.undo_label().map(label),
        redo_label: session.redo_label().map(label),
    }
}

/// `http://asset.localhost/<encoded path>`: what Tauri's `convertFileSrc` makes on Windows.
pub fn asset_url(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::from("http://asset.localhost/");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---- edits ----

/// Aspect id -> canvas size, as `ui/src/model.ts` ASPECTS.
pub fn aspect_size(aspect: &str) -> Option<(u32, u32)> {
    Some(match aspect {
        "16:9" => (1920, 1080),
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        "4:5" => (1080, 1350),
        _ => return None,
    })
}

pub const FONTS: [&str; 3] = ["Space Grotesk", "Inter", "JetBrains Mono"];

fn id_of(v: &Value, key: &str) -> ApiResult<Id> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::new("not_found", "That item is no longer in the project."))
}

fn ticks(v: &Value, key: &str) -> ApiResult<Tick> {
    opt_ticks(v, key)?.ok_or_else(|| invalid("a time is missing"))
}

fn opt_ticks(v: &Value, key: &str) -> ApiResult<Option<Tick>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x
            .as_f64()
            .filter(|f| f.is_finite() && f.abs() < 1e18)
            .map(|f| Some(Tick(f.round() as i64)))
            .ok_or_else(|| invalid("times must be numbers")),
    }
}

fn text(v: &Value, key: &str) -> ApiResult<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.chars().take(500).collect())),
        _ => Err(invalid("text must be a string")),
    }
}

/// Translate one UI edit into the core commands that make it (each one undo step; almost always
/// exactly one).
pub fn translate(p: &Project, name: &str, a: &Value) -> ApiResult<Vec<Command>> {
    let tl = &p.timeline;
    let frame = p.settings.fps.frame_duration();
    let main_at = |at: Tick| p.clip_at(p.settings.fps.snap_nearest(at));
    Ok(match name {
        "project.rename" => vec![Command::Rename {
            name: text(a, "name")?.unwrap_or_default(),
        }],
        "project.set_canvas" => {
            let aspect = text(a, "aspect")?.unwrap_or_default();
            let (width, height) = aspect_size(&aspect).ok_or_else(|| invalid("unknown aspect"))?;
            vec![Command::SetCanvas {
                aspect,
                width,
                height,
            }]
        }
        "timeline.append" | "timeline.insert" => {
            let media = id_of(a, "media_id")?;
            let m = p
                .media(media)
                .ok_or_else(|| ApiError::new("not_found", "That media is gone."))?;
            let end = if m.meta.kind == MediaKind::Image {
                Tick(5 * TICKS_PER_SECOND)
            } else {
                m.meta.duration
            };
            if m.meta.kind == MediaKind::Audio {
                vec![Command::AddMusic {
                    media,
                    start: Tick::ZERO,
                    source_in: Tick::ZERO,
                    source_out: end,
                }]
            } else {
                let index = (name == "timeline.insert")
                    .then(|| a.get("index").and_then(Value::as_u64))
                    .flatten()
                    .map(|i| (i as usize).min(tl.main.len()));
                vec![Command::InsertClip {
                    media,
                    source_in: Tick::ZERO,
                    source_out: p.settings.fps.snap(end),
                    index,
                }]
            }
        }
        "clip.split" => {
            let at = ticks(a, "at")?;
            match a
                .get("clip_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<Id>().ok())
            {
                Some(id) if tl.titles.iter().any(|t| t.id == id) => {
                    vec![Command::SplitTitle { id, at }]
                }
                Some(id) if tl.music.iter().any(|m| m.id == id) => {
                    vec![Command::SplitMusic { id, at }]
                }
                _ => vec![Command::Split { at }],
            }
        }
        "clip.trim" => {
            let id = id_of(a, "clip_id")?;
            let delta = p.settings.fps.snap_nearest(ticks(a, "delta")?);
            let inn = a.get("edge").and_then(Value::as_str) == Some("in");
            if let Some(c) = tl.main.iter().find(|c| c.id == id) {
                if inn {
                    vec![Command::TrimStart {
                        id,
                        source_in: c.source_in + delta,
                        drag: None,
                    }]
                } else {
                    vec![Command::TrimEnd {
                        id,
                        source_out: c.source_out + delta,
                        drag: None,
                    }]
                }
            } else if let Some(c) = tl.music.iter().find(|c| c.id == id) {
                if inn {
                    // Music keeps its place: the start moves with the in point (never before 0).
                    let lo = (c.source_in - c.start).max(Tick::ZERO);
                    let v = (c.source_in + delta).clamp(lo, c.source_out - frame);
                    vec![Command::EditMusic {
                        id,
                        start: Some(c.start + (v - c.source_in)),
                        source_in: Some(v),
                        source_out: None,
                    }]
                } else {
                    vec![Command::EditMusic {
                        id,
                        start: None,
                        source_in: None,
                        source_out: Some((c.source_out + delta).max(c.source_in + frame)),
                    }]
                }
            } else if let Some(t) = tl.titles.iter().find(|t| t.id == id) {
                let (start, duration) = if inn {
                    let v = delta.clamp(-t.start, t.duration - frame);
                    (t.start + v, t.duration - v)
                } else {
                    (t.start, (t.duration + delta).max(frame))
                };
                vec![edit_title(
                    id,
                    Some(start),
                    Some(duration),
                    None,
                    None,
                    None,
                )]
            } else {
                return Err(ApiError::new("not_found", "That clip is gone."));
            }
        }
        "clip.ripple_trim" => {
            let at = ticks(a, "at")?;
            let (_, start, c) = main_at(at)
                .ok_or_else(|| invalid("Move the playhead inside a clip to trim it."))?;
            let at = p.settings.fps.snap_nearest(at);
            if at == start {
                return Err(invalid("Move the playhead inside a clip to trim it."));
            }
            if a.get("edge").and_then(Value::as_str) == Some("start") {
                vec![Command::TrimStart {
                    id: c.id,
                    source_in: c.source_in + (at - start),
                    drag: None,
                }]
            } else {
                vec![Command::TrimEnd {
                    id: c.id,
                    source_out: c.source_in + (at - start),
                    drag: None,
                }]
            }
        }
        "clip.delete" => {
            let id = id_of(a, "clip_id")?;
            if tl.main.iter().any(|c| c.id == id) {
                vec![Command::RippleDelete { id }]
            } else if tl.titles.iter().any(|c| c.id == id) {
                vec![Command::DeleteTitle { id }]
            } else if tl.music.iter().any(|c| c.id == id) {
                vec![Command::DeleteMusic { id }]
            } else {
                return Err(ApiError::new("not_found", "That clip is gone."));
            }
        }
        "clip.move" => {
            let id = id_of(a, "clip_id")?;
            let i = tl
                .main
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| ApiError::new("not_found", "That clip is gone."))?;
            let want = a.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            // The UI's index counts the moved clip's old slot.
            let index = if want > i { want - 1 } else { want };
            vec![Command::MoveClip {
                id,
                index: index.min(tl.main.len().saturating_sub(1)),
            }]
        }
        "title.add" => vec![Command::AddTitle {
            start: ticks(a, "start")?.max(Tick::ZERO),
            duration: ticks(a, "duration")?,
            text: text(a, "text")?.unwrap_or_default(),
            style: Some(TitleStyle {
                font: "Space Grotesk".into(),
                size: 72.0,
                color: [255, 255, 255, 255],
            }),
            position: Some([0.5, 0.82]),
        }],
        "title.update" => {
            let id = id_of(a, "id")?;
            let t = tl
                .titles
                .iter()
                .find(|t| t.id == id)
                .ok_or_else(|| ApiError::new("not_found", "That title is gone."))?;
            let (style, position) = match a.get("style") {
                Some(s @ Value::Object(_)) => style_patch(t, s)?,
                _ => (None, None),
            };
            let start = opt_ticks(a, "start")?.map(|s| s.max(Tick::ZERO));
            vec![edit_title(
                id,
                start,
                None,
                text(a, "text")?,
                style,
                position,
            )]
        }
        "music.add" => {
            let media = id_of(a, "media_id")?;
            let m = p
                .media(media)
                .ok_or_else(|| ApiError::new("not_found", "That media is gone."))?;
            if !m.meta.has_audio || m.meta.kind == MediaKind::Image {
                return Err(ApiError::new(
                    "no_audio",
                    "That file has no sound to use as music.",
                ));
            }
            vec![Command::AddMusic {
                media,
                start: ticks(a, "start")?.max(Tick::ZERO),
                source_in: Tick::ZERO,
                source_out: m.meta.duration,
            }]
        }
        "music.update" => {
            let id = id_of(a, "id")?;
            let c = tl
                .music
                .iter()
                .find(|c| c.id == id)
                .ok_or_else(|| ApiError::new("not_found", "That music is gone."))?;
            let mut out = Vec::new();
            if let Some(start) = opt_ticks(a, "start")? {
                out.push(Command::EditMusic {
                    id,
                    start: Some(start.max(Tick::ZERO)),
                    source_in: None,
                    source_out: None,
                });
            }
            let gain = a.get("gain_db").and_then(Value::as_f64);
            let (fi, fo) = (opt_ticks(a, "fade_in")?, opt_ticks(a, "fade_out")?);
            if gain.is_some() || fi.is_some() || fo.is_some() {
                let len = c.duration();
                let fade_in = fi.unwrap_or(c.fade_in).clamp(Tick::ZERO, len);
                let fade_out = fo.unwrap_or(c.fade_out).clamp(Tick::ZERO, len - fade_in);
                out.push(Command::SetMusicGain {
                    id,
                    gain_db: gain.map_or(c.gain_db, |g| g.clamp(-60.0, 12.0) as f32),
                    fade_in,
                    fade_out,
                });
            }
            out
        }
        other => {
            return Err(ApiError::new(
                "unknown_command",
                format!("Unknown command {other}."),
            ));
        }
    })
}

fn edit_title(
    id: Id,
    start: Option<Tick>,
    duration: Option<Tick>,
    text: Option<String>,
    style: Option<TitleStyle>,
    position: Option<[f32; 2]>,
) -> Command {
    Command::EditTitle {
        id,
        start,
        duration,
        text,
        style,
        position,
    }
}

fn style_patch(t: &TitleClip, s: &Value) -> ApiResult<(Option<TitleStyle>, Option<[f32; 2]>)> {
    let mut style = t.style.clone();
    if let Some(f) = s.get("font").and_then(Value::as_str) {
        style.font = FONTS
            .iter()
            .find(|x| **x == f)
            .ok_or_else(|| invalid("unknown font"))?
            .to_string();
    }
    if let Some(sz) = s.get("size").and_then(Value::as_f64) {
        style.size = (sz as f32).clamp(8.0, 600.0);
    }
    if let Some(c) = s.get("color").and_then(Value::as_str) {
        style.color = parse_hex(c).ok_or_else(|| invalid("colours look like #rrggbb"))?;
    }
    let position = match s.get("position").and_then(Value::as_str) {
        Some(name) => Some(
            POSITIONS
                .iter()
                .find(|x| x.0 == name)
                .map(|x| [0.5, x.1])
                .ok_or_else(|| invalid("unknown title position"))?,
        ),
        None => None,
    };
    Ok((Some(style), position))
}

#[cfg(test)]
mod tests {
    use super::*;
    use editelier_core::{MediaMeta, Settings};
    use serde_json::json;

    fn session() -> Session {
        let mut s = Session::new(Project::new(Settings::default()));
        s.execute(&Command::AddMedia {
            path: "a.mp4".into(),
            meta: MediaMeta {
                kind: MediaKind::Video,
                duration: Tick(10 * TICKS_PER_SECOND),
                fps: Some(FrameRate::FPS_30),
                is_vfr: false,
                has_audio: true,
            },
        })
        .unwrap();
        s
    }

    fn run(s: &mut Session, name: &str, a: Value) {
        for c in translate(s.project(), name, &a).unwrap() {
            s.execute(&c).unwrap();
        }
    }

    #[test]
    fn ui_edits_map_to_core() {
        let mut s = session();
        run(&mut s, "timeline.append", json!({"media_id": "1"}));
        run(&mut s, "timeline.append", json!({"media_id": "1"}));
        let sec = TICKS_PER_SECOND;
        run(&mut s, "clip.split", json!({"at": 3 * sec}));
        assert_eq!(s.project().timeline.main.len(), 3);
        let last = s.project().timeline.main[2].id.to_string();
        run(&mut s, "clip.move", json!({"clip_id": last, "index": 0}));
        run(
            &mut s,
            "title.add",
            json!({"start": 0, "duration": 2 * sec, "text": "Hi"}),
        );
        let t = s.project().timeline.titles[0].id.to_string();
        run(
            &mut s,
            "title.update",
            json!({"id": t, "style": {"position": "top", "color": "#ff0000", "font": "Inter"}}),
        );
        let title = &s.project().timeline.titles[0];
        assert_eq!(
            (title.position, title.style.color),
            ([0.5, 0.15], [255, 0, 0, 255])
        );
        run(
            &mut s,
            "clip.trim",
            json!({"clip_id": t, "edge": "in", "delta": sec}),
        );
        assert_eq!(s.project().timeline.titles[0].start, Tick(sec));
        run(
            &mut s,
            "clip.split",
            json!({"at": sec + sec / 2, "clip_id": t}),
        );
        assert_eq!(s.project().timeline.titles.len(), 2);
        run(&mut s, "music.add", json!({"media_id": "1", "start": sec}));
        let m = s.project().timeline.music[0].id.to_string();
        run(
            &mut s,
            "music.update",
            json!({"id": m, "gain_db": -6, "fade_in": sec, "fade_out": 99 * sec}),
        );
        let mc = &s.project().timeline.music[0];
        assert_eq!(
            (mc.gain_db, mc.fade_in, mc.fade_out),
            (-6.0, Tick(sec), Tick(9 * sec))
        );
        run(&mut s, "project.set_canvas", json!({"aspect": "9:16"}));
        assert_eq!(s.project().settings.width, 1080);
        assert!(translate(s.project(), "format.disk", &json!({})).is_err());
        assert!(translate(s.project(), "clip.delete", &json!({"clip_id": "x"})).is_err());
        let lib = Library::default();
        let snap = serde_json::to_value(snapshot(&s, None, 1, &lib)).unwrap();
        assert_eq!(snap["project"]["canvas"]["aspect"], "9:16");
        assert_eq!(
            snap["project"]["timeline"]["titles"][0]["style"]["position"],
            "top"
        );
        assert_eq!(snap["undo_label"], "Change canvas");
    }

    #[test]
    fn asset_urls_encode_paths() {
        assert_eq!(
            asset_url(std::path::Path::new(r"C:\a b\t.bmp")),
            "http://asset.localhost/C%3A%5Ca%20b%5Ct.bmp"
        );
    }
}
