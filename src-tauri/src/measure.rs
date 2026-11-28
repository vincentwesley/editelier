//! `--measure`: scripted playback bench, prints stats, then exits (≈ 45 s, no lingering window).
//! Phase 1: a 1080p30 H.264 timeline from the originals for 10 s. Phase 2: a 4K60 H.264 clip on a
//! 60 fps timeline through its auto proxy for 20 s (A-PERF-02 asks ≥ 30 fps). Media come from
//! `scripts/make-test-media.ps1` (`test-media/`), or `EDITELIER_MEASURE_DIR`.
//! `EDITELIER_MEASURE_SECS=60` plays each phase that long (A-PERF-01/02, `cargo xtask accept`).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::AppState;
use crate::library::Proxy;
use editelier_core::{FrameRate, Project, Session, Settings, Tick};
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

pub struct Summary {
    pub fps: f64,
    pub mean: f64,
    pub p99: f64,
}

pub fn summarize(intervals_ms: &[f64]) -> Summary {
    let n = intervals_ms.len().max(1) as f64;
    let sum: f64 = intervals_ms.iter().sum();
    Summary {
        fps: if sum > 0.0 { n * 1000.0 / sum } else { 0.0 },
        mean: sum / n,
        p99: pct(intervals_ms, 0.99),
    }
}

/// Process CPU time (all threads of this process; WebView2 runs in its own processes).
pub struct CpuClock {
    wall: Instant,
    cpu: u64,
}

impl CpuClock {
    pub fn now() -> Self {
        Self {
            wall: Instant::now(),
            cpu: process_cpu_100ns(),
        }
    }
    /// CPU since the last lap as % of one core, then restart the lap.
    pub fn lap_percent_of_core(&mut self) -> f64 {
        let n = Self::now();
        let pct =
            (n.cpu - self.cpu) as f64 / 1e4 / (n.wall - self.wall).as_secs_f64().max(1e-3) / 10.0;
        *self = n;
        pct
    }
}

fn process_cpu_100ns() -> u64 {
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    ft(k) + ft(u)
}

fn media_dir() -> PathBuf {
    std::env::var_os("EDITELIER_MEASURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test-media"))
}

fn pct(v: &[f64], q: f64) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(((v.len() as f64 * q) as usize).min(v.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0)
}

pub fn run(app: &AppHandle) -> bool {
    let t_start = Instant::now();
    let state = app.state::<AppState>();
    let core = state.core.clone();
    let win = app.get_webview_window("main").expect("main window");
    let _ = win.set_focus();
    // The viewer rect normally comes from the editor UI; place it ourselves (1280x720).
    if let Some(p) = state.preview.lock().unwrap().as_ref() {
        p.set_rect(crate::preview::Rect {
            x: 0,
            y: 0,
            w: 1280,
            h: 720,
            dpr: 1.0,
        });
    }
    let dir = media_dir();
    let mut ok = true;
    println!(
        "{:<16} {:>7} {:>9} {:>9} {:>9} {:>7} {:>10} {:>10} {:>10} {:>6}",
        "phase",
        "secs",
        "video fps",
        "target",
        "present",
        "p99 ms",
        "upload ms",
        "decode ms",
        "dec p95",
        "cpu%"
    );
    let secs_or = |quick: f64| {
        std::env::var("EDITELIER_MEASURE_SECS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(quick)
    };
    // `clip_secs`: fixture length; enough copies go on the timeline to outlast the phase.
    let phase =
        |name: &str, file: &str, fps: FrameRate, clip_secs: f64, secs: f64, proxies: bool| {
            let copies = (secs / clip_secs).ceil() as usize + 1;
            let path = dir.join(file);
            if !path.is_file() {
                println!(
                    "SKIP {name}: {} missing (scripts/make-test-media.ps1)",
                    path.display()
                );
                return None;
            }
            let p = Project::new(Settings {
                fps,
                ..Settings::default()
            });
            core.replace_doc(Session::new(p), None);
            if let Err(e) = core.import(Some(app), &[path.display().to_string()]) {
                println!("FAIL {name}: import: {}", e.message);
                return None;
            }
            let id = *core.lib.read().unwrap().keys().next()?;
            if proxies {
                let t = Instant::now();
                loop {
                    let proxy = core.lib.read().unwrap().get(&id).map(|e| e.proxy.clone());
                    match proxy {
                        Some(Proxy::Ready { .. }) => break,
                        Some(Proxy::Failed(e)) => {
                            println!("FAIL {name}: proxy: {e}");
                            return None;
                        }
                        _ if t.elapsed() > Duration::from_secs(90) => {
                            println!("FAIL {name}: proxy timed out");
                            return None;
                        }
                        _ => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
                println!(
                    "  {name}: proxy ready after {:.1} s",
                    t.elapsed().as_secs_f64()
                );
            }
            core.engine.use_proxies.store(proxies, Ordering::Relaxed);
            for _ in 0..copies {
                let args = serde_json::json!({"media_id": id.to_string()});
                if let Err(e) = core.run_command("timeline.append", &args) {
                    println!("FAIL {name}: {}", e.message);
                    return None;
                }
            }
            core.engine.seek(Tick::ZERO);
            std::thread::sleep(Duration::from_millis(400)); // first frames decoded
            std::mem::take(&mut *state.acc.lock().unwrap());
            std::mem::take(&mut *core.engine.stats.lock().unwrap());
            let mut cpu = CpuClock::now();
            let t = Instant::now();
            core.engine.play(1.0);
            std::thread::sleep(Duration::from_secs_f64(secs));
            let wall = t.elapsed().as_secs_f64();
            core.engine.play(0.0);
            let cpu_pct = cpu.lap_percent_of_core();
            std::thread::sleep(Duration::from_millis(150)); // let the 100 ms drain catch up
            let w = std::mem::take(&mut *state.acc.lock().unwrap());
            let d = std::mem::take(&mut *core.engine.stats.lock().unwrap());
            let s = summarize(&w.intervals_ms);
            let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
            let video_fps = w.uploads as f64 / wall;
            println!(
                "{:<16} {:>7.1} {:>9.1} {:>9.2} {:>9.1} {:>7.1} {:>10.2} {:>10.2} {:>10.2} {:>6.1}",
                name,
                wall,
                video_fps,
                fps.as_f64(),
                s.fps,
                s.p99,
                avg(&w.upload_ms),
                avg(&d.decode_ms),
                pct(&d.decode_ms, 0.95),
                cpu_pct
            );
            Some(video_fps)
        };
    if let Some(f) = phase(
        "1080p30 original",
        "cfr30_1080p_h264_aac.mp4",
        FrameRate::FPS_30,
        4.0,
        secs_or(10.0),
        false,
    ) {
        ok &= f >= 29.5; // A-PERF-01 (docs/ACCEPTANCE.md; xtask verdict judges)
    }
    if let Some(f) = phase(
        "4K60 via proxy",
        "uhd60_h264.mp4",
        FrameRate::FPS_60,
        3.0,
        secs_or(20.0),
        true,
    ) {
        ok &= f >= 30.0;
    }
    println!("total {:.1} s; ok: {ok}", t_start.elapsed().as_secs_f64());
    ok
}
