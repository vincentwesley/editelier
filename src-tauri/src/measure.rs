//! `--measure`: scripted ≤20 s run over window states, prints frame stats, then exits.

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, LogicalSize, Manager};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetDC, GetPixel, ReleaseDC};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_F24,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GW_CHILD, GW_HWNDNEXT, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindow, GetWindowThreadProcessId, PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP,
};

use crate::AppState;

pub struct Summary {
    pub n: usize,
    pub fps: f64,
    pub mean: f64,
    pub p99: f64,
    pub max: f64,
}

pub fn summarize(intervals_ms: &[f64]) -> Summary {
    if intervals_ms.is_empty() {
        return Summary {
            n: 0,
            fps: 0.0,
            mean: 0.0,
            p99: 0.0,
            max: 0.0,
        };
    }
    let mut v = intervals_ms.to_vec();
    v.sort_by(f64::total_cmp);
    let sum: f64 = v.iter().sum();
    let n = v.len();
    Summary {
        n,
        fps: n as f64 * 1000.0 / sum,
        mean: sum / n as f64,
        p99: v[((n as f64 * 0.99) as usize).min(n - 1)],
        max: v[n - 1],
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

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn focus_class(parent: HWND) -> String {
    let tid = unsafe { GetWindowThreadProcessId(parent, None) };
    let mut gi = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    match unsafe { GetGUIThreadInfo(tid, &mut gi) } {
        Ok(()) if !gi.hwndFocus.is_invalid() => class_of(gi.hwndFocus),
        _ => "(none)".into(),
    }
}

/// Screen pixel at the preview's probe square (magenta = our HWND is on top and presenting).
fn probe(parent: HWND, x: i32, y: i32) -> (u8, u8, u8) {
    unsafe {
        let mut pt = POINT { x: x + 8, y: y + 8 };
        let _ = ClientToScreen(parent, &mut pt);
        let dc = GetDC(None);
        let c = GetPixel(dc, pt.x, pt.y).0;
        ReleaseDC(None, dc);
        (c as u8, (c >> 8) as u8, (c >> 16) as u8)
    }
}

pub fn run(app: &AppHandle) -> bool {
    let t_start = Instant::now();
    let state = app.state::<AppState>();
    let win = app.get_webview_window("main").expect("main window");
    let (shared, phwnd) = {
        let g = state.preview.lock().unwrap();
        let p = g.as_ref().expect("preview");
        (p.shared.clone(), HWND(p.parent as _))
    };
    while shared.rect_updates.load(Ordering::Relaxed) == 0 {
        if t_start.elapsed() > Duration::from_secs(8) {
            println!("FAIL: UI never reported a preview rect");
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    println!(
        "first rect after {:.0} ms",
        t_start.elapsed().as_secs_f64() * 1e3
    );
    println!(
        "window DPI {} ({}%)",
        unsafe { GetDpiForWindow(phwnd) },
        unsafe { GetDpiForWindow(phwnd) } * 100 / 96
    );
    let mut z = vec![];
    let mut c = unsafe { GetWindow(phwnd, GW_CHILD) }.ok();
    while let Some(h) = c {
        z.push(class_of(h));
        c = unsafe { GetWindow(h, GW_HWNDNEXT) }.ok();
    }
    println!("child z-order (top first): {}", z.join(" > "));

    println!(
        "{:<12} {:>6} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>6} {:>5} {:>4} {:>11} {:>14}",
        "phase",
        "frames",
        "fps",
        "mean_ms",
        "p99_ms",
        "max_ms",
        "acq_ms",
        "work_ms",
        "cpu%",
        "rects",
        "cfg",
        "size",
        "probe rgb"
    );
    let mut steady_ok = false;
    let mut phase = |name: &str, secs: f64, action: &dyn Fn()| {
        std::mem::take(&mut *state.acc.lock().unwrap());
        let rects0 = shared.rect_updates.load(Ordering::Relaxed);
        let mut cpu = CpuClock::now();
        let t = Instant::now();
        action();
        let left = Duration::from_secs_f64(secs).saturating_sub(t.elapsed());
        std::thread::sleep(left);
        let cpu_pct = cpu.lap_percent_of_core();
        std::thread::sleep(Duration::from_millis(120)); // let the 100 ms drain catch up
        let w = std::mem::take(&mut *state.acc.lock().unwrap());
        let s = summarize(&w.intervals_ms);
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let r = *shared.rect.lock().unwrap();
        let px = probe(phwnd, r.x, r.y);
        if name == "steady" {
            steady_ok = s.fps > 20.0 && px == (255, 0, 255);
        }
        println!(
            "{:<12} {:>6} {:>7.1} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.3} {:>6.1} {:>5} {:>4} {:>11} {:>14}",
            name,
            s.n,
            s.fps,
            s.mean,
            s.p99,
            s.max,
            avg(&w.acquire_ms),
            avg(&w.cpu_ms),
            cpu_pct,
            shared.rect_updates.load(Ordering::Relaxed) - rects0,
            w.reconfigures,
            format!("{}x{}", r.w, r.h),
            format!("{px:?}"),
        );
    };

    // Bring the window forward so the screen probe isn't sampling whatever covers it.
    phase("warmup", 1.5, &|| {
        let _ = win.set_focus();
    });
    phase("steady", 3.0, &|| {});
    let lat = Mutex::new(vec![]);
    phase("resize", 3.0, &|| {
        // Window resize → ResizeObserver → invoke → SetWindowPos latency.
        for i in 0..28u32 {
            let k = (i % 7) as f64;
            let n0 = shared.rect_updates.load(Ordering::Relaxed);
            let t = Instant::now();
            let _ = win.set_size(LogicalSize::new(1280.0 + k * 40.0, 720.0 + k * 25.0));
            while shared.rect_updates.load(Ordering::Relaxed) == n0
                && t.elapsed() < Duration::from_millis(95)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            lat.lock().unwrap().push(t.elapsed().as_secs_f64() * 1e3);
            std::thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
        }
    });
    let l = summarize(&lat.lock().unwrap());
    println!(
        "  resize→rect latency over {} steps: mean {:.1} ms, p99 {:.1} ms, max {:.1} ms",
        l.n, l.mean, l.p99, l.max
    );
    phase("maximized", 2.5, &|| {
        let _ = win.maximize();
    });
    phase("restored", 1.5, &|| {
        let _ = win.unmaximize();
    });
    phase("minimized", 2.0, &|| {
        let _ = win.minimize();
    });
    phase("unminimized", 2.5, &|| {
        let _ = win.unminimize();
        let _ = win.set_focus();
    });

    // Keyboard: a click on the preview must not take focus; F24 must reach the page.
    let preview_hwnd = HWND(state.preview.lock().unwrap().as_ref().unwrap().hwnd as _);
    let before = focus_class(phwnd);
    unsafe {
        let _ = PostMessageW(
            Some(preview_hwnd),
            WM_LBUTTONDOWN,
            WPARAM(1),
            LPARAM(0x0010_0010),
        );
        let _ = PostMessageW(
            Some(preview_hwnd),
            WM_LBUTTONUP,
            WPARAM(0),
            LPARAM(0x0010_0010),
        );
    }
    std::thread::sleep(Duration::from_millis(150));
    let after = focus_class(phwnd);
    let foreground = unsafe { GetForegroundWindow() } == phwnd;
    if foreground {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_F24,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        unsafe {
            SendInput(
                &[key(Default::default()), key(KEYEVENTF_KEYUP)],
                size_of::<INPUT>() as i32,
            )
        };
        std::thread::sleep(Duration::from_millis(500));
    }
    println!(
        "focus before click: {before}; after click on preview: {after}; foreground: {foreground}; F24 reached page: {}",
        state.key_seen.load(Ordering::Relaxed)
    );
    println!(
        "total {:.1} s; steady ok: {steady_ok}",
        t_start.elapsed().as_secs_f64()
    );
    steady_ok
}
