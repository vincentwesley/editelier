//! Tauri commands per the UI contract (`ui/src/api/commands.ts`). Every path or value from the UI
//! is untrusted: paths are canonicalised, must exist and have an allowed extension; the only files
//! written are the chosen project/export paths and the app's own data and cache folders.

use crate::api::{self, ApiError, ApiResult, ProxyState, Snapshot, invalid};
use crate::engine::Engine;
use crate::export;
use crate::library::{self, Library, Proxy};
use editelier_core::{Command, Id, Project, ProjectInfo, Session, Settings, Tick};
use editelier_export::CancelToken;
use editelier_media::{Tools, cache, ffmpeg, needs_proxy, win};
use editelier_render::Gpu;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct Doc {
    pub id: u64,
    pub session: Session,
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

pub struct Dirs {
    pub cache: PathBuf,
    pub data: PathBuf,
    pub videos: Option<PathBuf>,
    pub documents: Option<PathBuf>,
}

/// Everything the commands share. One per app.
pub struct Core {
    pub doc: Mutex<Option<Doc>>,
    pub lib: Library,
    pub engine: Arc<Engine>,
    pub tools: Option<Tools>,
    pub dirs: Dirs,
    pub gpu: Option<Gpu>,
    jobs: Mutex<HashMap<String, CancelToken>>,
    /// Bumped when the project is replaced: background work for the old one stops.
    generation: AtomicU64,
    counter: AtomicU64,
}

pub const PROJECT_EXT: &str = "editelier";
const PRESETS: [&str; 6] = [
    "youtube-1080",
    "youtube-4k",
    "tiktok",
    "instagram-reel",
    "instagram-45",
    "master",
];
const KEYMAPS: [&str; 3] = ["editelier", "premiere", "resolve"];

/// `\\?\C:\x` -> `C:\x` (canonicalize's verbatim form confuses Explorer and users).
pub fn plain(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

fn ext_is(p: &Path, want: &str) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(want))
}

fn bad_path(raw: &str) -> bool {
    raw.is_empty() || raw.len() > 4096 || raw.contains('\0')
}

/// An existing directory, canonical.
fn existing_dir(p: &Path) -> ApiResult<PathBuf> {
    let d = std::fs::canonicalize(p).map_err(|_| invalid("That folder doesn't exist."))?;
    if !d.is_dir() {
        return Err(invalid("That is not a folder."));
    }
    Ok(plain(d))
}

/// A file name Windows accepts, no folders in it.
fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c))
        && !name.ends_with(['.', ' '])
}

impl Core {
    pub fn new(dirs: Dirs, gpu: Option<Gpu>) -> Arc<Self> {
        let lib = Library::default();
        let engine = Engine::new(lib.clone());
        if let Some(i) = gpu
            .as_ref()
            .and_then(|g| editelier_render::Interop::new(g).ok())
        {
            let _ = engine.interop.set(i);
        }
        Arc::new(Core {
            doc: Mutex::new(None),
            engine,
            lib,
            tools: Tools::locate().map_err(|e| eprintln!("{e}")).ok(),
            dirs,
            gpu,
            jobs: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            counter: AtomicU64::new(1),
        })
    }

    fn tools(&self) -> ApiResult<&Tools> {
        self.tools.as_ref().ok_or_else(|| {
            ApiError::new(
                "ffmpeg_missing",
                "The FFmpeg helper is missing from the Editelier folder. Reinstall Editelier.",
            )
        })
    }

    fn snap(&self, d: &Doc) -> Snapshot {
        api::snapshot(&d.session, d.path.as_deref(), d.id, &self.lib)
    }

    fn with_doc<T>(&self, f: impl FnOnce(&mut Doc) -> ApiResult<T>) -> ApiResult<T> {
        let mut g = self.doc.lock().unwrap();
        let d = g
            .as_mut()
            .ok_or_else(|| ApiError::new("no_project", "No project is open."))?;
        f(d)
    }

    /// After an edit: playback sees the new project, autosave picks it up.
    fn changed(&self, d: &mut Doc) {
        d.dirty = true;
        self.engine.set_project(d.session.project().clone());
    }

    pub(crate) fn replace_doc(&self, session: Session, path: Option<PathBuf>) -> Snapshot {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let d = Doc {
            id: self.counter.fetch_add(1, Ordering::Relaxed),
            session,
            path,
            dirty: false,
        };
        self.engine.set_project(d.session.project().clone());
        self.engine.seek(Tick::ZERO);
        let s = self.snap(&d);
        *self.doc.lock().unwrap() = Some(d);
        s
    }

    pub fn run_command(&self, name: &str, args: &Value) -> ApiResult<Snapshot> {
        self.with_doc(|d| {
            match name {
                "history.undo" => {
                    d.session
                        .undo()
                        .ok_or_else(|| ApiError::new("nothing_to_undo", "Nothing to undo."))?;
                }
                "history.redo" => {
                    d.session
                        .redo()
                        .ok_or_else(|| ApiError::new("nothing_to_redo", "Nothing to redo."))?;
                }
                _ => {
                    for c in api::translate(d.session.project(), name, args)? {
                        d.session.execute(&c)?;
                    }
                }
            }
            self.changed(d);
            Ok(self.snap(d))
        })
    }

    pub fn project_new(
        &self,
        name: &str,
        aspect: &str,
        preset: &str,
        keymap: &str,
    ) -> ApiResult<Snapshot> {
        let (width, height) =
            api::aspect_size(aspect).ok_or_else(|| invalid("Unknown canvas shape."))?;
        let name = name.trim();
        let mut p = Project::new(Settings {
            width,
            height,
            fps: editelier_core::FrameRate::FPS_29_97,
            ..Settings::default()
        });
        p.info = ProjectInfo {
            name: if name.is_empty() {
                "Untitled project".into()
            } else {
                name.chars().take(120).collect()
            },
            aspect: aspect.into(),
            export_preset: if PRESETS.contains(&preset) {
                preset
            } else {
                "master"
            }
            .into(),
            keymap: if KEYMAPS.contains(&keymap) {
                keymap
            } else {
                "editelier"
            }
            .into(),
        };
        self.lib.write().unwrap().clear();
        Ok(self.replace_doc(Session::new(p), None))
    }

    /// Probe + index + thumbnail now; audio and proxies in the background.
    pub fn import(
        self: &Arc<Self>,
        app: Option<&AppHandle>,
        paths: &[String],
    ) -> ApiResult<Snapshot> {
        if paths.is_empty() || paths.len() > 500 {
            return Err(invalid("Pick between 1 and 500 files to import."));
        }
        let checked: Vec<PathBuf> = paths
            .iter()
            .map(|p| library::check_media_path(p).map(plain))
            .collect::<ApiResult<_>>()?;
        let tools = self.tools()?.clone();
        let mut loaded = Vec::new();
        let mut failed = None;
        for (i, path) in checked.iter().enumerate() {
            match library::load(&tools, &self.dirs.cache, path) {
                Ok(e) => loaded.push(e),
                Err(e) => failed = Some(e),
            }
            if let Some(app) = app {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                let _ = app.emit(
                    "import-progress",
                    serde_json::json!({"done": i + 1, "total": checked.len(), "name": name}),
                );
            }
        }
        if loaded.is_empty() {
            return Err(failed.unwrap_or_else(|| invalid("Nothing to import.")));
        }
        let (snap, ids) = self.with_doc(|d| {
            let mut ids = Vec::new();
            for e in loaded {
                let meta = library::meta_of(&e.probe, e.index.as_ref());
                let id = d
                    .session
                    .execute(&Command::AddMedia {
                        path: e.path.display().to_string(),
                        meta,
                    })?
                    .ok_or_else(|| invalid("no media id"))?;
                self.lib.write().unwrap().insert(id, Arc::new(e));
                ids.push(id);
            }
            self.changed(d);
            Ok((self.snap(d), ids))
        })?;
        self.background(app.cloned(), ids);
        Ok(snap)
    }

    /// Audio decode (+ waveform) and proxies for `ids`, one at a time, on a worker thread.
    fn background(self: &Arc<Self>, app: Option<AppHandle>, ids: Vec<Id>) {
        let me = self.clone();
        let generation = self.generation.load(Ordering::Relaxed);
        std::thread::spawn(move || {
            let Ok(tools) = me.tools().cloned() else {
                return;
            };
            let alive = || me.generation.load(Ordering::Relaxed) == generation;
            let emit = |id: Id, p: &Proxy| {
                if let Some(app) = &app {
                    let _ = app.emit(
                        "proxy-progress",
                        serde_json::json!({"media_id": id.to_string(), "proxy": ProxyState::from(p)}),
                    );
                }
            };
            for id in ids {
                let Some(e) = me.lib.read().unwrap().get(&id).cloned() else {
                    continue;
                };
                if !alive() {
                    return;
                }
                if !e.probe.audio.is_empty() {
                    match library::load_audio(&tools, &me.dirs.cache, &e) {
                        Ok(a) => library::update(&me.lib, id, |x| x.audio = Some(a)),
                        Err(err) => eprintln!("audio {}: {}", e.path.display(), err.message),
                    }
                }
                let (Some(v), Some(ix)) = (e.probe.video.first(), e.index.as_ref()) else {
                    continue;
                };
                if !needs_proxy(v, ix) {
                    continue;
                }
                library::update(&me.lib, id, |x| x.proxy = Proxy::Building(0.0));
                emit(id, &Proxy::Building(0.0));
                let device = win::create_device().ok();
                let mut last = 0.0;
                let res = win::make_proxy(
                    &e.path,
                    v,
                    e.probe.duration_us.unwrap_or(1),
                    &me.dirs.cache,
                    device.as_ref(),
                    &mut |f| {
                        if f - last >= 0.05 {
                            last = f;
                            library::update(&me.lib, id, |x| x.proxy = Proxy::Building(f));
                            emit(id, &Proxy::Building(f));
                        }
                        alive()
                    },
                );
                let proxy = res.and_then(|path| {
                    let probe = editelier_media::probe(&tools, &path)?;
                    let pv = probe.video.first().cloned().ok_or_else(|| {
                        editelier_media::Error::Codec("proxy has no video".into())
                    })?;
                    let index = ffmpeg::index(&tools, &path, &pv, &me.dirs.cache)?;
                    Ok(Proxy::Ready {
                        path,
                        video: Box::new(pv),
                        index,
                    })
                });
                let proxy = match proxy {
                    Ok(p) => p,
                    Err(editelier_media::Error::Cancelled) => Proxy::None,
                    Err(err) => Proxy::Failed(err.to_string()),
                };
                library::update(&me.lib, id, |x| x.proxy = proxy.clone());
                emit(id, &proxy);
                me.engine.invalidate();
            }
        });
    }

    pub fn open(self: &Arc<Self>, app: Option<&AppHandle>, raw: &str) -> ApiResult<Snapshot> {
        if bad_path(raw) {
            return Err(invalid("That is not a file path."));
        }
        let path = std::fs::canonicalize(raw)
            .map(plain)
            .map_err(|_| ApiError::new("not_found", "That project file was moved or deleted."))?;
        let meta = std::fs::metadata(&path)?;
        if !ext_is(&path, PROJECT_EXT) || !meta.is_file() || meta.len() > 64 << 20 {
            return Err(invalid("That isn't an Editelier project file."));
        }
        let text = std::fs::read_to_string(&path)?;
        let p = editelier_core::from_json(&text)?;
        let tools = self.tools()?.clone();
        let mut entries = HashMap::new();
        for m in p.media.iter() {
            // Missing or unreadable media stays in the project; it just shows nothing.
            // Local disk only (never a share): see `check_project_media_path`.
            if let Ok(e) = library::check_project_media_path(&m.path)
                .and_then(|path| library::load(&tools, &self.dirs.cache, &plain(path)))
            {
                entries.insert(m.id, Arc::new(e));
            }
        }
        let ids: Vec<Id> = entries.keys().copied().collect();
        *self.lib.write().unwrap() = entries;
        let snap = self.replace_doc(Session::new(p), Some(path.clone()));
        self.add_recent(&path);
        self.background(app.cloned(), ids);
        Ok(snap)
    }

    pub fn save(&self, raw: Option<&str>) -> ApiResult<Snapshot> {
        let chosen = match raw {
            Some(r) => {
                if bad_path(r) {
                    return Err(invalid("That is not a file path."));
                }
                let p = Path::new(r);
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !ext_is(p, PROJECT_EXT) || !file_name_ok(name) {
                    return Err(invalid("Project files end in .editelier."));
                }
                let parent = p.parent().ok_or_else(|| invalid("Pick a folder."))?;
                Some(existing_dir(parent)?.join(name))
            }
            None => None,
        };
        let (snap, path) = self.with_doc(|d| {
            let path = match chosen.or_else(|| d.path.clone()) {
                Some(p) => p,
                None => {
                    let docs = self
                        .dirs
                        .documents
                        .clone()
                        .ok_or_else(|| invalid("No Documents folder to save into."))?
                        .join("Editelier");
                    std::fs::create_dir_all(&docs)?;
                    let base: String = d
                        .session
                        .project()
                        .info
                        .name
                        .chars()
                        .filter(|c| file_name_ok(&c.to_string()))
                        .collect();
                    let base = if base.trim().is_empty() {
                        "Untitled".into()
                    } else {
                        base
                    };
                    unique(&docs, base.trim(), PROJECT_EXT)
                }
            };
            cache::write_atomic(
                &path,
                editelier_core::to_json(d.session.project()).as_bytes(),
            )?;
            d.path = Some(path.clone());
            d.dirty = false;
            Ok((self.snap(d), path))
        })?;
        self.add_recent(&path);
        Ok(snap)
    }

    fn recent_file(&self) -> PathBuf {
        self.dirs.data.join("recent.json")
    }

    pub fn recent(&self) -> Vec<Recent> {
        let list: Vec<Recent> = std::fs::read(self.recent_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        list.into_iter()
            .filter(|r| Path::new(&r.path).is_file())
            .collect()
    }

    fn add_recent(&self, path: &Path) {
        let name = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| editelier_core::from_json(&t).ok())
            .map_or_else(|| "Untitled project".into(), |p| p.info.name.clone());
        let entry = Recent {
            name,
            path: path.display().to_string(),
            modified: iso_now(),
        };
        let mut list = self.recent();
        list.retain(|r| r.path != entry.path);
        list.insert(0, entry);
        list.truncate(10);
        if let Ok(json) = serde_json::to_vec_pretty(&list) {
            let _ = cache::write_atomic(&self.recent_file(), &json);
        }
    }

    /// Autosave to the app data folder (never over the user's file) when there are edits.
    pub fn autosave(&self) -> Option<String> {
        let mut g = self.doc.lock().unwrap();
        let d = g.as_mut().filter(|d| d.dirty)?;
        let file = self
            .dirs
            .data
            .join("autosave")
            .join(format!("{}.{PROJECT_EXT}", d.id));
        cache::write_atomic(
            &file,
            editelier_core::to_json(d.session.project()).as_bytes(),
        )
        .ok()?;
        d.dirty = false;
        Some(iso_now())
    }

    /// Validates an export destination: `.mp4`, an existing folder (or one under the user's
    /// Videos folder, created), never overwriting.
    pub fn export_path(&self, raw: &str) -> ApiResult<PathBuf> {
        if bad_path(raw) {
            return Err(invalid("That is not a file path."));
        }
        let p = Path::new(raw);
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !ext_is(p, "mp4") || !file_name_ok(name) {
            return Err(invalid("Exports are .mp4 files with a plain name."));
        }
        let parent = p.parent().unwrap_or(Path::new(""));
        let dir = if parent.is_absolute() {
            existing_dir(parent)?
        } else {
            // "Videos\..." from the UI's default folder: only below the user's Videos folder.
            let mut parts = parent.components();
            let videos = self
                .dirs
                .videos
                .clone()
                .ok_or_else(|| invalid("Pick a folder."))?;
            match parts.next() {
                Some(Component::Normal(first)) if first.eq_ignore_ascii_case("Videos") => {}
                _ => return Err(invalid("Pick a folder to export into.")),
            }
            let rest: PathBuf = parts
                .map(|c| match c {
                    Component::Normal(s) => Ok(s),
                    _ => Err(invalid("Pick a folder to export into.")),
                })
                .collect::<ApiResult<_>>()?;
            let d = videos.join(rest);
            std::fs::create_dir_all(&d)?;
            existing_dir(&d)?
        };
        let stem = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Export");
        Ok(unique(&dir, stem, "mp4"))
    }

    pub fn export_start(
        self: &Arc<Self>,
        app: Option<AppHandle>,
        preset: &str,
        raw_path: &str,
        range: Option<(f64, f64)>,
    ) -> ApiResult<String> {
        let path = self.export_path(raw_path)?;
        let gpu = self.gpu.clone().ok_or_else(|| {
            ApiError::new("no_gpu", "Exporting needs a DirectX 12 graphics card.")
        })?;
        let tools = self.tools()?.clone();
        let project = self.with_doc(|d| Ok(d.session.project().clone()))?;
        if project.timeline.main.is_empty() && project.timeline.titles.is_empty() {
            return Err(ApiError::new(
                "empty_timeline",
                "Add a clip to the timeline before exporting.",
            ));
        }
        let preset = export::preset_for(preset, &project)?;
        let end = project.duration();
        let fps = project.settings.fps;
        let (a, b) = range.unwrap_or((0.0, f64::MAX));
        let clamp = |v: f64| Tick(v.clamp(0.0, end.0 as f64) as i64);
        let (start, stop) = (fps.snap(clamp(a)), fps.snap_nearest(clamp(b)));
        if stop <= start {
            return Err(invalid("The export range is empty."));
        }
        let job_id = format!("job{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let cancel = CancelToken::default();
        self.jobs
            .lock()
            .unwrap()
            .insert(job_id.clone(), cancel.clone());
        let job = export::Job {
            media: self.lib.read().unwrap().clone(),
            project,
            preset,
            path: path.clone(),
            range: (start, stop),
            cancel,
        };
        let (me, id) = (self.clone(), job_id.clone());
        std::thread::spawn(move || {
            let mut pct = -1i32;
            let res = export::run(job, tools, &gpu, |f| {
                let now = (f * 100.0) as i32;
                if now != pct {
                    pct = now;
                    if let Some(app) = &app {
                        let _ = app.emit(
                            "export-progress",
                            serde_json::json!({"job": id, "progress": now}),
                        );
                    }
                }
            });
            me.jobs.lock().unwrap().remove(&id);
            if let Some(app) = &app {
                let _ = match res {
                    Ok(done) => {
                        eprintln!(
                            "export: {} frames in {:.1} s with {}",
                            done.frames, done.seconds, done.encoder
                        );
                        app.emit(
                            "export-done",
                            serde_json::json!({"job": id, "path": path.display().to_string()}),
                        )
                    }
                    Err(e) => app.emit("export-failed", serde_json::json!({"job": id, "error": e})),
                };
            }
        });
        Ok(job_id)
    }

    pub fn export_cancel(&self, job: &str) {
        if let Some(c) = self.jobs.lock().unwrap().get(job) {
            c.cancel();
        }
    }
}

/// `dir/stem.ext`, or `dir/stem (2).ext` and so on when taken.
fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    p
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        if e.raw_os_error() == Some(112) {
            return ApiError::new("disk_full", "The disk is full.");
        }
        ApiError::new("io_error", e.to_string())
    }
}

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct Recent {
    name: String,
    path: String,
    modified: String,
}

/// UTC now as ISO 8601 (no date crate: days-from-civil inverse).
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

// ---- Tauri command wrappers ----

type Res<T> = Result<T, ApiError>;

fn core(state: &State<'_, crate::AppState>) -> Arc<Core> {
    state.core.clone()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::new("internal", e.to_string()))?
}

#[tauri::command]
pub async fn run_command(
    state: State<'_, crate::AppState>,
    name: String,
    args: Value,
) -> Res<Snapshot> {
    let c = core(&state);
    blocking(move || c.run_command(&name, &args)).await
}

#[tauri::command]
pub async fn project_new(
    state: State<'_, crate::AppState>,
    name: String,
    aspect: String,
    export_preset: String,
    keymap: String,
) -> Res<Snapshot> {
    core(&state).project_new(&name, &aspect, &export_preset, &keymap)
}

#[tauri::command]
pub async fn project_open(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    path: String,
) -> Res<Snapshot> {
    let c = core(&state);
    blocking(move || c.open(Some(&app), &path)).await
}

#[tauri::command]
pub async fn project_save(
    state: State<'_, crate::AppState>,
    path: Option<String>,
) -> Res<Snapshot> {
    let c = core(&state);
    blocking(move || c.save(path.as_deref())).await
}

#[tauri::command]
pub async fn recent_projects(state: State<'_, crate::AppState>) -> Res<Vec<Recent>> {
    Ok(core(&state).recent())
}

#[tauri::command]
pub async fn import_media(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    paths: Vec<String>,
) -> Res<Snapshot> {
    let c = core(&state);
    blocking(move || c.import(Some(&app), &paths)).await
}

fn dialog(app: &AppHandle) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    use tauri_plugin_dialog::DialogExt;
    let d = app.dialog().file();
    match app.get_webview_window("main") {
        Some(w) => d.set_parent(&w),
        None => d,
    }
}

#[tauri::command]
pub async fn dialog_open_media(app: AppHandle) -> Res<Option<Vec<String>>> {
    blocking(move || {
        let files = dialog(&app)
            .add_filter("Video, audio and photos", library::MEDIA_EXTS)
            .blocking_pick_files();
        Ok(files.map(|v| v.iter().map(|f| f.to_string()).collect()))
    })
    .await
}

#[tauri::command]
pub async fn dialog_open_project(app: AppHandle) -> Res<Option<String>> {
    blocking(move || {
        let f = dialog(&app)
            .add_filter("Editelier project", &[PROJECT_EXT])
            .blocking_pick_file();
        Ok(f.map(|f| f.to_string()))
    })
    .await
}

#[tauri::command]
pub async fn dialog_pick_folder(app: AppHandle) -> Res<Option<String>> {
    if let Some(dir) = test_export_dir() {
        return Ok(Some(dir));
    }
    blocking(move || Ok(dialog(&app).blocking_pick_folder().map(|f| f.to_string()))).await
}

/// E2E hook (D7): the native folder picker can't be automated, so debug + `agent` builds run
/// with `EDITELIER_AGENT=1` answer it with `EDITELIER_TEST_EXPORT_DIR`. Compiled out of every
/// other build; the chosen folder still goes through `export_start`'s path checks.
fn test_export_dir() -> Option<String> {
    #[cfg(all(feature = "agent", debug_assertions))]
    if std::env::var("EDITELIER_AGENT").as_deref() == Ok("1") {
        return std::env::var("EDITELIER_TEST_EXPORT_DIR")
            .ok()
            .filter(|d| !d.is_empty());
    }
    None
}

#[derive(Serialize)]
pub struct EncoderLabel {
    label: String,
    hardware: bool,
}

#[tauri::command]
pub async fn export_encoder() -> Res<EncoderLabel> {
    blocking(|| {
        let list = editelier_export::win::enumerate_encoders(editelier_export::VideoCodec::H264)
            .map_err(|e| ApiError::new("export_failed", e.to_string()))?;
        let first = list
            .first()
            .ok_or_else(|| ApiError::new("no_encoder", "No H.264 encoder found."))?;
        let vendor = match first.vendor {
            editelier_export::Vendor::Intel => "Intel Quick Sync",
            editelier_export::Vendor::Amd => "AMD",
            editelier_export::Vendor::Nvidia => "NVIDIA",
            editelier_export::Vendor::OtherHardware => "graphics card",
            editelier_export::Vendor::Software => "Windows software encoder",
        };
        let kind = if first.hardware {
            "hardware"
        } else {
            "software"
        };
        Ok(EncoderLabel {
            label: format!("Auto: {vendor} ({kind})"),
            hardware: first.hardware,
        })
    })
    .await
}

#[derive(serde::Deserialize)]
pub struct Range {
    start: f64,
    end: f64,
}

#[tauri::command]
pub async fn export_start(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    preset: String,
    path: String,
    range: Option<Range>,
) -> Res<String> {
    let c = core(&state);
    blocking(move || c.export_start(Some(app), &preset, &path, range.map(|r| (r.start, r.end))))
        .await
}

#[tauri::command]
pub async fn export_cancel(state: State<'_, crate::AppState>, job: String) -> Res<()> {
    core(&state).export_cancel(&job);
    Ok(())
}

#[tauri::command]
pub async fn reveal_path(path: String) -> Res<()> {
    if bad_path(&path) {
        return Err(invalid("That is not a file path."));
    }
    let p = std::fs::canonicalize(&path)
        .map(plain)
        .map_err(|_| ApiError::new("not_found", "That file was moved or deleted."))?;
    let mut cmd = std::process::Command::new("explorer.exe");
    if p.is_file() {
        cmd.arg(format!("/select,{}", p.display()));
    } else {
        cmd.arg(&p);
    }
    cmd.spawn()?;
    Ok(())
}

#[tauri::command]
pub async fn transport_play(state: State<'_, crate::AppState>, rate: f64) -> Res<()> {
    if !rate.is_finite() || rate.abs() > 8.0 {
        return Err(invalid("Playback speed must be between -8 and 8."));
    }
    core(&state).engine.play(rate);
    Ok(())
}

#[tauri::command]
pub async fn transport_seek(state: State<'_, crate::AppState>, ticks: f64) -> Res<()> {
    if !ticks.is_finite() {
        return Err(invalid("That is not a time."));
    }
    core(&state).engine.seek(Tick(ticks.max(0.0) as i64));
    Ok(())
}

/// Emits `playhead` at ~30 Hz while playing (and once when it stops) and autosaves every 3 s.
pub fn tick_loop(app: AppHandle, c: Arc<Core>) {
    std::thread::spawn(move || {
        let mut last = (Tick(-1), false);
        let mut n = 0u64;
        while !c.engine.stopped() {
            std::thread::sleep(std::time::Duration::from_millis(33));
            let now = c.engine.position();
            if now != last && (now.1 || last.1 || now.0 != last.0) {
                let _ = app.emit(
                    "playhead",
                    serde_json::json!({"ticks": now.0 .0, "playing": now.1}),
                );
                last = now;
            }
            n += 1;
            if n.is_multiple_of(90)
                && let Some(at) = c.autosave()
            {
                let _ = app.emit("autosaved", serde_json::json!({"at": at}));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_dir_only_in_debug_agent_builds() {
        // SAFETY: no other test reads these variables.
        unsafe {
            std::env::set_var("EDITELIER_AGENT", "1");
            std::env::set_var("EDITELIER_TEST_EXPORT_DIR", r"C:\e2e");
        }
        let want = cfg!(all(feature = "agent", debug_assertions)).then(|| r"C:\e2e".to_string());
        assert_eq!(test_export_dir(), want);
        unsafe { std::env::set_var("EDITELIER_AGENT", "0") };
        assert_eq!(test_export_dir(), None);
    }

    fn core() -> Arc<Core> {
        let tmp = std::env::temp_dir().join("editelier-commands-test");
        std::fs::create_dir_all(&tmp).unwrap();
        Core::new(
            Dirs {
                cache: tmp.join("cache"),
                data: tmp.join("data"),
                videos: Some(tmp.join("videos")),
                documents: Some(tmp.join("docs")),
            },
            None,
        )
    }

    #[test]
    fn export_paths_are_confined() {
        let c = core();
        assert!(c.export_path("Videos\\out.mp4").is_ok());
        assert!(c.export_path("Videos\\Editelier\\out.mp4").is_ok());
        assert!(c.export_path("Videos\\..\\evil.mp4").is_err());
        assert!(c.export_path("..\\evil.mp4").is_err());
        assert!(c.export_path("C:\\nowhere-at-all\\x.mp4").is_err());
        assert!(c.export_path("Videos\\out.exe").is_err());
        assert!(c.export_path("Videos\\a:b.mp4").is_err());
        let dir = std::env::temp_dir();
        let p = c.export_path(&format!("{}\\x.mp4", dir.display())).unwrap();
        assert!(
            p.ends_with("x.mp4") || p.file_name().unwrap().to_str().unwrap().starts_with("x (")
        );
    }

    #[test]
    fn project_round_trip_and_bad_inputs() {
        let c = core();
        assert_eq!(
            c.run_command("history.undo", &Value::Null)
                .err()
                .unwrap()
                .code,
            "no_project"
        );
        assert!(c.project_new("Trip", "21:9", "x", "y").is_err());
        let s =
            serde_json::to_value(c.project_new("Trip", "9:16", "evil", "vim").unwrap()).unwrap();
        assert_eq!(s["project"]["export_preset"], "master");
        assert_eq!(s["project"]["keymap"], "editelier");
        c.run_command("project.rename", &serde_json::json!({"name": "Holiday"}))
            .unwrap();
        assert!(c.save(Some("C:\\x\\..\\..\\evil.exe")).is_err());
        let file = std::env::temp_dir()
            .join("editelier-commands-test")
            .join("t.editelier");
        let _ = std::fs::remove_file(&file);
        c.save(Some(file.to_str().unwrap())).unwrap();
        assert!(c.open(None, "C:\\Windows\\win.ini").is_err());
        let s = serde_json::to_value(c.open(None, file.to_str().unwrap()).unwrap()).unwrap();
        assert_eq!(s["project"]["name"], "Holiday");
        assert!(c.recent().iter().any(|r| r.path.ends_with("t.editelier")));
        assert!(iso_now().ends_with('Z') && iso_now().len() == 20);
    }

    #[test]
    fn project_with_share_media_opens_offline() {
        use editelier_core::{MediaKind, MediaMeta, MediaRef};
        let c = core();
        if c.tools.is_none() {
            return eprintln!("SKIP: no FFmpeg helper");
        }
        let mut p = Project::new(Settings::default());
        // TEST-NET address: a filesystem call on it would stall for seconds, not fail fast.
        p.media = Arc::new(vec![MediaRef {
            id: "1".parse().unwrap(),
            path: r"\\192.0.2.1@SSL\share\a.mp4".into(),
            meta: MediaMeta {
                kind: MediaKind::Video,
                duration: Tick(1),
                fps: None,
                is_vfr: false,
                has_audio: false,
            },
        }]);
        let file = std::env::temp_dir()
            .join("editelier-commands-test")
            .join("unc.editelier");
        std::fs::write(&file, editelier_core::to_json(&p)).unwrap();
        let t = std::time::Instant::now();
        let s = serde_json::to_value(c.open(None, file.to_str().unwrap()).unwrap()).unwrap();
        assert!(t.elapsed().as_millis() < 1000, "touched the share");
        assert_eq!(s["project"]["media"][0]["offline"], true);
    }
}
