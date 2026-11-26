//! Agent control channel (D7): JSON-RPC 2.0 over a per-user named pipe, for E2E tests and
//! walkthroughs. Compiled only into debug builds with `--features agent`, started only when
//! `EDITELIER_AGENT=1`.
//!
//! Security: the pipe has a protected DACL granting only the current user, rejects remote
//! clients and must be the first instance of its random name (no squatting). Its name and a
//! 256-bit token go to `%TEMP%\editelier-agent.json` (same user-only DACL, deleted on exit).
//! Every request carries the token. Only the methods in [`METHODS`] exist; each has a strict
//! params struct. Paths are validated like the UI's (media/export) or confined to the temp dir
//! (screenshots). No file reads, no shell.

use crate::api::{ApiError, ApiResult, invalid};
use crate::commands::{Core, plain};
use crate::library;
use editelier_core::{Layer, Tick, TitleClip};
use editelier_render::{Fonts, Renderer};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, HWND, INVALID_HANDLE_VALUE, LocalFree, RECT,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_GENERIC_WRITE, FILE_SHARE_NONE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::{HSTRING, PWSTR};

/// The fixed allow-list. Anything else is "method not found".
pub const METHODS: [&str; 10] = [
    "run_command",
    "snapshot",
    "import_media",
    "seek",
    "play",
    "pause",
    "frame_hash",
    "export_start",
    "export_status",
    "screenshot",
];

const MAX_LINE: u64 = 1 << 20;
const INFO_FILE: &str = "editelier-agent.json";

#[derive(Deserialize, Debug)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Call {
    RunCommand {
        name: String,
        #[serde(default)]
        args: Value,
    },
    Snapshot {},
    ImportMedia {
        paths: Vec<String>,
    },
    Seek {
        ticks: f64,
    },
    Play {
        rate: f64,
    },
    Pause {},
    FrameHash {},
    ExportStart {
        preset: String,
        path: String,
    },
    ExportStatus {
        job: String,
    },
    Screenshot {
        path: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: String,
    id: Value,
    token: String,
    method: String,
    #[serde(default)]
    params: Value,
}

fn rpc_err(id: &Value, code: i32, message: &str, data: Option<&str>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message, "data": data}})
}

/// Constant time for equal lengths (the token length is public).
fn token_ok(given: &str, want: &str) -> bool {
    given.len() == want.len()
        && given
            .bytes()
            .zip(want.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// One request line -> one response, and whether to drop the connection (bad token).
pub fn handle_line(
    line: &str,
    token: &str,
    exec: impl FnOnce(Call) -> ApiResult<Value>,
) -> (Value, bool) {
    let Ok(req) = serde_json::from_str::<Request>(line) else {
        return (
            rpc_err(&Value::Null, -32600, "Invalid request", None),
            false,
        );
    };
    if !token_ok(&req.token, token) {
        return (rpc_err(&req.id, -32001, "Unauthorized", None), true);
    }
    if req.jsonrpc != "2.0" {
        return (rpc_err(&req.id, -32600, "Invalid request", None), false);
    }
    if !METHODS.contains(&req.method.as_str()) {
        return (rpc_err(&req.id, -32601, "Method not found", None), false);
    }
    let params = if req.params.is_null() {
        json!({})
    } else {
        req.params
    };
    let call = match serde_json::from_value::<Call>(json!({"method": req.method, "params": params}))
    {
        Ok(c) => c,
        Err(e) => return (rpc_err(&req.id, -32602, &e.to_string(), None), false),
    };
    let resp = match exec(call) {
        Ok(v) => json!({"jsonrpc": "2.0", "id": req.id, "result": v}),
        Err(e) => rpc_err(&req.id, -32000, &e.message, Some(&e.code)),
    };
    (resp, false)
}

fn random_hex<const N: usize>() -> String {
    let mut b = [0u8; N];
    unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .expect("system RNG");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `D:P(A;;GA;;;<current user SID>)`: protected DACL, the current user only.
struct UserOnly(PSECURITY_DESCRIPTOR);

impl UserOnly {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            let mut tok = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok)?;
            let mut buf = vec![0u8; 256];
            let mut len = 0;
            let r = GetTokenInformation(
                tok,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                buf.len() as u32,
                &mut len,
            );
            let _ = CloseHandle(tok);
            r?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut s)?;
            let sid = s.to_string().unwrap_or_default();
            LocalFree(Some(HLOCAL(s.0.cast())));
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(format!("D:P(A;;GA;;;{sid})")),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )?;
            Ok(UserOnly(sd))
        }
    }

    fn attrs(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for UserOnly {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

fn info_path() -> PathBuf {
    std::env::temp_dir().join(INFO_FILE)
}

/// Starts the channel when `EDITELIER_AGENT=1`. Errors are logged; the app runs without it.
pub fn start(app: &AppHandle) {
    if std::env::var("EDITELIER_AGENT").as_deref() != Ok("1") {
        return;
    }
    if std::env::var("EDITELIER_TEST_OFFSCREEN").as_deref() == Ok("1")
        && let Some(w) = app.get_webview_window("main")
    {
        let _ = w.set_position(tauri::PhysicalPosition::new(-32000, -32000));
    }
    let jobs = track_exports(app);
    let app = app.clone();
    std::thread::Builder::new()
        .name("agent".into())
        .spawn(move || {
            if let Err(e) = serve(&app, &jobs) {
                eprintln!("agent: {e}");
            }
        })
        .expect("agent thread");
}

pub fn stop() {
    let _ = std::fs::remove_file(info_path());
}

type Jobs = Arc<Mutex<HashMap<String, Value>>>;

/// Export status from the shell's own events, for UI- and agent-started jobs alike.
fn track_exports(app: &AppHandle) -> Jobs {
    let jobs: Jobs = Arc::default();
    for (event, state) in [
        ("export-progress", "running"),
        ("export-done", "done"),
        ("export-failed", "failed"),
    ] {
        let jobs = jobs.clone();
        app.listen_any(event, move |e| {
            let Ok(mut v) = serde_json::from_str::<Value>(e.payload()) else {
                return;
            };
            let Some(job) = v["job"].as_str().map(str::to_owned) else {
                return;
            };
            v["state"] = state.into();
            jobs.lock().unwrap().insert(job, v);
        });
    }
    jobs
}

fn serve(app: &AppHandle, jobs: &Jobs) -> std::io::Result<()> {
    let sd = UserOnly::new()?;
    let sa = sd.attrs();
    let name = format!(r"\\.\pipe\editelier-agent-{}", random_hex::<16>());
    let token = random_hex::<32>();
    let pipe = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name.as_str()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1 << 16,
            1 << 16,
            0,
            Some(&sa),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let pipe = unsafe { File::from_raw_handle(pipe.0) };

    let info = info_path();
    let _ = std::fs::remove_file(&info);
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(info.as_os_str()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            Some(&sa),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?
    };
    let mut f = unsafe { File::from_raw_handle(h.0) };
    f.write_all(
        json!({"pipe": name, "token": token, "pid": std::process::id()})
            .to_string()
            .as_bytes(),
    )?;
    drop(f);
    eprintln!("agent: listening ({})", info.display());

    let raw = HANDLE(pipe.as_raw_handle());
    loop {
        if let Err(e) = unsafe { ConnectNamedPipe(raw, None) }
            && e.code() != ERROR_PIPE_CONNECTED.to_hresult()
        {
            return Err(e.into());
        }
        let mut r = BufReader::new(&pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = r.by_ref().take(MAX_LINE).read_until(b'\n', &mut buf)?;
            if n == 0 || buf.last() != Some(&b'\n') {
                break; // client gone, or a line over the limit
            }
            let line = String::from_utf8_lossy(&buf);
            let (resp, drop_client) = handle_line(&line, &token, |c| exec(app, jobs, c));
            let mut out = resp.to_string().into_bytes();
            out.push(b'\n');
            (&pipe).write_all(&out)?;
            if drop_client {
                break;
            }
        }
        unsafe {
            let _ = FlushFileBuffers(raw);
            let _ = DisconnectNamedPipe(raw);
        }
    }
}

fn exec(app: &AppHandle, jobs: &Jobs, call: Call) -> ApiResult<Value> {
    let core = app.state::<crate::AppState>().core.clone();
    Ok(match call {
        Call::RunCommand { name, args } => to_json(&core.run_command(&name, &args)?),
        Call::Snapshot {} => {
            let g = core.doc.lock().unwrap();
            let d = g
                .as_ref()
                .ok_or_else(|| ApiError::new("no_project", "No project is open."))?;
            to_json(&crate::api::snapshot(
                &d.session,
                d.path.as_deref(),
                d.id,
                &core.lib,
            ))
        }
        Call::ImportMedia { paths } => {
            if paths.is_empty() || paths.len() > 500 {
                return Err(invalid("Pick between 1 and 500 files to import."));
            }
            let paths: Vec<String> = paths
                .iter()
                .map(|p| library::check_media_path(p).map(|p| plain(p).display().to_string()))
                .collect::<ApiResult<_>>()?;
            // Same path as a file dropped outside the timeline: the UI imports it itself.
            app.emit(
                "tauri://drag-drop",
                json!({"paths": paths, "position": {"x": -1.0, "y": -1.0}}),
            )
            .map_err(|e| ApiError::new("internal", e.to_string()))?;
            json!({"queued": paths.len()})
        }
        Call::Seek { ticks } => {
            if !ticks.is_finite() {
                return Err(invalid("That is not a time."));
            }
            core.engine.seek(Tick(ticks.max(0.0) as i64));
            Value::Null
        }
        Call::Play { rate } => {
            if !rate.is_finite() || rate.abs() > 8.0 {
                return Err(invalid("Playback speed must be between -8 and 8."));
            }
            core.engine.play(rate);
            Value::Null
        }
        Call::Pause {} => {
            core.engine.play(0.0);
            Value::Null
        }
        Call::FrameHash {} => frame_hash(&core)?,
        Call::ExportStart { preset, path } => {
            json!({"job": core.export_start(Some(app.clone()), &preset, &path, None)?})
        }
        Call::ExportStatus { job } => jobs
            .lock()
            .unwrap()
            .get(&job)
            .cloned()
            .unwrap_or_else(|| json!({"job": job, "state": "unknown"})),
        Call::Screenshot { path } => {
            let path = screenshot_path(&path)?;
            let w = app
                .get_webview_window("main")
                .ok_or_else(|| invalid("no window"))?;
            let hwnd = w
                .hwnd()
                .map_err(|e| ApiError::new("internal", e.to_string()))?;
            let (width, height) = screenshot(hwnd, &path)?;
            json!({"path": path.display().to_string(), "width": width, "height": height})
        }
    })
}

fn to_json(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// The preview canvas for the clock time, composed like the viewer does and read back once.
fn frame_hash(core: &Core) -> ApiResult<Value> {
    let gpu = core
        .gpu
        .clone()
        .ok_or_else(|| ApiError::new("no_gpu", "No GPU."))?;
    let engine = &core.engine;
    // Wait (bounded) for the decoder to park the exact frame for the clock.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (t, hit) = loop {
        let (t, hit) = engine.display();
        let want = engine.project().settings.fps.frame_at(t);
        if hit.as_ref().is_some_and(|(f, _)| *f == want) || Instant::now() > deadline {
            break (t, hit);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut r = Renderer::new(&gpu, Fonts::bundled());
    match hit.and_then(|(_, s)| s) {
        Some((_, f)) => r
            .upload(&f.source())
            .map_err(|e| ApiError::new("render", e.to_string()))?,
        None => r.clear_clip(),
    }
    let p = engine.project();
    let layers = p.layers_at(t);
    let titles: Vec<&TitleClip> = layers
        .iter()
        .filter_map(|l| match l {
            Layer::Title(t) => Some(*t),
            _ => None,
        })
        .collect();
    let ((w, h), scale) = crate::preview::canvas_size(p.settings.width, p.settings.height);
    r.compose((w, h), &titles, scale);
    let px = r
        .read_rgba8()
        .map_err(|e| ApiError::new("render", e.to_string()))?;
    // FNV-1a 64: stable across runs and Rust versions.
    let hash = px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    Ok(json!({"hash": format!("{hash:016x}"), "ticks": t.0, "width": w, "height": h}))
}

/// A new `.bmp` directly inside (or below) the user's temp folder; nothing else.
fn screenshot_path(raw: &str) -> ApiResult<PathBuf> {
    if raw.is_empty() || raw.len() > 1024 || raw.contains('\0') {
        return Err(invalid("That is not a file path."));
    }
    let p = Path::new(raw);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let plain_name = !name.is_empty()
        && name.len() <= 255
        && !name.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c));
    let is_bmp = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("bmp"));
    if !plain_name || !is_bmp {
        return Err(invalid("Screenshots are .bmp files with a plain name."));
    }
    let temp = std::fs::canonicalize(std::env::temp_dir())?;
    let dir = p
        .parent()
        .and_then(|d| std::fs::canonicalize(d).ok())
        .ok_or_else(|| invalid("That folder doesn't exist."))?;
    if !dir.starts_with(&temp) {
        return Err(invalid("Screenshots go in the temp folder."));
    }
    Ok(plain(dir).join(name))
}

/// PrintWindow (full content, so WebView2 is included) into a 32-bit top-down BMP.
fn screenshot(hwnd: HWND, path: &Path) -> ApiResult<(u32, u32)> {
    let fail = |m: &str| ApiError::new("screenshot", m.to_string());
    let mut rc = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rc) }.map_err(|e| fail(&e.to_string()))?;
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    if w <= 0 || h <= 0 || w > 16_384 || h > 16_384 {
        return Err(fail("bad window size"));
    }
    let mut px = vec![0u8; w as usize * h as usize * 4];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let ok = unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        // PW_RENDERFULLCONTENT (not in the metadata).
        let printed = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        SelectObject(dc, old);
        let rows = GetDIBits(
            dc,
            bmp,
            0,
            h as u32,
            Some(px.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        printed && rows == h
    };
    if !ok {
        return Err(fail("PrintWindow failed"));
    }
    let size = 54 + px.len() as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(-h).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&px);
    // create_new: never overwrites (and never follows a planted link).
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(&out)?;
    Ok((w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "ab12";

    fn req(token: &str, method: &str, params: Value) -> String {
        json!({"jsonrpc": "2.0", "id": 7, "token": token, "method": method, "params": params})
            .to_string()
    }

    fn run(line: &str) -> (Value, bool, Option<String>) {
        let mut seen = None;
        let (v, drop) = handle_line(line, TOKEN, |c| {
            seen = Some(format!("{c:?}"));
            Ok(json!("ok"))
        });
        (v, drop, seen)
    }

    #[test]
    fn token_is_required() {
        for t in ["", "ab13", "ab1", "ab12x", "AB12"] {
            let (v, drop, seen) = run(&req(t, "snapshot", json!({})));
            assert_eq!(v["error"]["code"], -32001, "{t}");
            assert!(drop && seen.is_none());
        }
        let no_token = json!({"jsonrpc": "2.0", "id": 1, "method": "snapshot"}).to_string();
        let (v, _, seen) = run(&no_token);
        assert_eq!(v["error"]["code"], -32600);
        assert!(seen.is_none());
        let (v, drop, seen) = run(&req(TOKEN, "snapshot", Value::Null));
        assert_eq!(v["result"], "ok");
        assert!(!drop && seen.unwrap().contains("Snapshot"));
        assert!(token_ok("abc", "abc") && !token_ok("abd", "abc"));
    }

    #[test]
    fn only_allow_listed_methods_with_strict_params() {
        for m in ["project_open", "eval", "shell", "read_file", "Snapshot", ""] {
            let (v, _, seen) = run(&req(TOKEN, m, json!({})));
            assert_eq!(v["error"]["code"], -32601, "{m}");
            assert!(seen.is_none());
        }
        // Unknown or wrongly typed params are rejected before anything runs.
        for (m, p) in [
            ("seek", json!({"ticks": "0"})),
            ("seek", json!({"ticks": 0, "extra": 1})),
            ("screenshot", json!({})),
            ("snapshot", json!({"x": 1})),
        ] {
            let (v, _, seen) = run(&req(TOKEN, m, p));
            assert_eq!(v["error"]["code"], -32602, "{m}");
            assert!(seen.is_none());
        }
        let (v, _, seen) = run(&req(TOKEN, "seek", json!({"ticks": 5})));
        assert_eq!(v["id"], 7);
        assert!(seen.unwrap().contains("Seek"));
        // Extra envelope fields and bad versions are invalid requests.
        let (v, _, _) = run(&req(TOKEN, "snapshot", json!({})).replace("\"2.0\"", "\"1.0\""));
        assert_eq!(v["error"]["code"], -32600);
        let (v, _, _) = run("not json");
        assert_eq!(v["error"]["code"], -32600);
    }

    #[test]
    fn errors_carry_the_api_code() {
        let (v, _) = handle_line(&req(TOKEN, "pause", json!({})), TOKEN, |_| {
            Err(ApiError::new("no_project", "No project is open."))
        });
        assert_eq!(v["error"]["data"], "no_project");
    }

    #[test]
    fn screenshots_stay_in_temp() {
        let temp = std::env::temp_dir();
        assert!(screenshot_path(&temp.join("a.bmp").display().to_string()).is_ok());
        assert!(screenshot_path(&temp.join("a.exe").display().to_string()).is_err());
        assert!(screenshot_path(&temp.join("..").join("a.bmp").display().to_string()).is_err());
        assert!(screenshot_path("C:\\Windows\\a.bmp").is_err());
        assert!(screenshot_path("a.bmp").is_err());
        assert!(screenshot_path("").is_err());
    }
}
