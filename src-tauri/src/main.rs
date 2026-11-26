#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod commands;
mod engine;
mod export;
mod library;
mod measure;
mod mix;
mod preview;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use commands::{Core, Dirs};
use preview::{Preview, Rect};
use tauri::{Emitter, Manager, RunEvent};

pub struct AppState {
    pub preview: Mutex<Option<Preview>>,
    /// Frames drained from the render thread every 100 ms; `measure` swaps it per phase.
    pub acc: Arc<Mutex<preview::Window>>,
    pub key_seen: std::sync::atomic::AtomicBool,
    /// `--measure` places the viewer itself; UI rect reports are ignored then.
    pub pin_rect: std::sync::atomic::AtomicBool,
    pub core: Arc<Core>,
}

#[tauri::command]
fn preview_set_rect(state: tauri::State<'_, AppState>, rect: Rect) {
    if state.pin_rect.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Some(p) = state.preview.lock().unwrap().as_ref() {
        p.set_rect(rect);
    }
}

#[tauri::command]
fn ui_key(state: tauri::State<'_, AppState>, key: String) {
    if key == "F24" {
        state
            .key_seen
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone, serde::Serialize)]
struct UiStats {
    fps: f64,
    frame_ms: f64,
    cpu: f64,
    w: u32,
    h: u32,
}

fn main() {
    let measure = std::env::args().any(|a| a == "--measure");
    // Never make sound in benches or automated runs: the audio clock runs on a silent sink.
    let null_audio = measure || std::env::var("EDITELIER_AUDIO").is_ok_and(|v| v == "null");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            preview_set_rect,
            ui_key,
            commands::run_command,
            commands::project_new,
            commands::project_open,
            commands::project_save,
            commands::recent_projects,
            commands::import_media,
            commands::dialog_open_media,
            commands::dialog_open_project,
            commands::dialog_pick_folder,
            commands::export_encoder,
            commands::export_start,
            commands::export_cancel,
            commands::reveal_path,
            commands::transport_play,
            commands::transport_seek,
        ])
        .setup(move |app| {
            let path = app.path();
            let local = path.app_local_data_dir()?;
            let dirs = Dirs {
                cache: local.join("cache"),
                data: local.join("data"),
                videos: path.video_dir().ok(),
                documents: path.document_dir().ok(),
            };
            let gpu = editelier_render::Gpu::headless()
                .map_err(|e| eprintln!("gpu: {e}"))
                .ok();
            let core = Core::new(dirs, gpu.clone());
            app.manage(AppState {
                preview: Mutex::new(None),
                acc: Arc::new(Mutex::new(preview::Window::default())),
                key_seen: false.into(),
                pin_rect: measure.into(),
                core: core.clone(),
            });
            if let Some(tools) = core.tools.clone() {
                let e = core.engine.clone();
                std::thread::Builder::new()
                    .name("decode-ahead".into())
                    .spawn(move || e.decode_loop(tools))?;
            }
            let e = core.engine.clone();
            std::thread::Builder::new()
                .name("audio-out".into())
                .spawn(move || e.audio_loop(null_audio))?;
            commands::tick_loop(app.handle().clone(), core.clone());

            let win = app.get_webview_window("main").expect("main window");
            let mut p = Preview::create(win.hwnd()?.0 as isize)?;
            if let Some(gpu) = gpu {
                p.start(gpu, core.engine.clone());
            }
            let shared = p.shared.clone();
            let state = app.state::<AppState>();
            *state.preview.lock().unwrap() = Some(p);

            // Drain render samples every 100 ms; push UI stats every 500 ms.
            let (acc, handle) = (state.acc.clone(), app.handle().clone());
            std::thread::spawn(move || {
                let mut ui = preview::Window::default();
                let mut cpu = measure::CpuClock::now();
                let mut tick = 0u64;
                loop {
                    tick += 1;
                    std::thread::sleep(Duration::from_millis(100));
                    let w = std::mem::take(&mut *shared.frames.lock().unwrap());
                    ui.intervals_ms.extend_from_slice(&w.intervals_ms);
                    acc.lock().unwrap().merge(w);
                    if tick.is_multiple_of(5) {
                        let r = *shared.rect.lock().unwrap();
                        let s = measure::summarize(&ui.intervals_ms);
                        let _ = handle.emit(
                            "preview-stats",
                            UiStats {
                                fps: s.fps,
                                frame_ms: s.mean,
                                cpu: cpu.lap_percent_of_core(),
                                w: r.w,
                                h: r.h,
                            },
                        );
                        ui = preview::Window::default();
                    }
                }
            });

            if measure {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let ok = measure::run(&handle);
                    handle.exit(if ok { 0 } else { 1 });
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, ev| {
            if let RunEvent::Exit = ev {
                let state = app.state::<AppState>();
                state.core.engine.shutdown();
                if let Some(mut p) = state.preview.lock().unwrap().take() {
                    p.shutdown();
                }
            }
        });
}
