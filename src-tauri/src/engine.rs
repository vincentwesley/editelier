//! Playback: transport + clock, a decode-ahead thread and audio output.
//!
//! The clock is the audio device (cpal/WASAPI sample count) while playing at 1× with audio on
//! the timeline, otherwise a monotonic timer × rate. The decode thread walks the timeline ahead
//! of the clock (core `layers_at`), decodes frames (proxies when ready) and parks them in a small
//! ring keyed by timeline frame; the preview thread shows the ring's frame for the clock time.
//! J/K/L: any rate in [-8, 8]; reverse and fast rates decode per frame (they may step).

use crate::library::{Decoders, Library, OwnedFrame};
use crate::mix;
use editelier_core::{Id, Layer, Project, Tick};
use editelier_media::Tools;
use editelier_render::Interop;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// What to show at one timeline frame: the decoded picture of `media`, or nothing (black).
pub type Shown = Option<(Id, Arc<OwnedFrame>)>;

struct Transport {
    playing: bool,
    rate: f64,
    anchor: Tick,
    anchor_at: Instant,
    audio_clock: bool,
}

#[derive(Default)]
pub struct DecodeStats {
    pub decode_ms: Vec<f64>,
}

pub struct Engine {
    project: RwLock<Arc<Project>>,
    pub lib: Library,
    pub use_proxies: AtomicBool,
    transport: Mutex<Transport>,
    /// Next timeline sample the audio device will be handed.
    audio_cursor: Arc<AtomicI64>,
    audio_running: Arc<AtomicBool>,
    audio_live: AtomicBool,
    ring: Mutex<BTreeMap<i64, Shown>>,
    wake: Condvar,
    stop: AtomicBool,
    pub stats: Mutex<DecodeStats>,
    /// Zero-copy decode into the preview's GPU; set once before the decode loop starts.
    pub interop: OnceLock<Interop>,
}

const RING: usize = 12;

impl Engine {
    pub fn new(lib: Library) -> Arc<Self> {
        Arc::new(Engine {
            project: RwLock::new(Arc::new(Project::default())),
            lib,
            use_proxies: AtomicBool::new(true),
            transport: Mutex::new(Transport {
                playing: false,
                rate: 0.0,
                anchor: Tick::ZERO,
                anchor_at: Instant::now(),
                audio_clock: false,
            }),
            audio_cursor: Arc::new(AtomicI64::new(0)),
            audio_running: Arc::new(AtomicBool::new(false)),
            audio_live: AtomicBool::new(false),
            ring: Mutex::new(BTreeMap::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            stats: Mutex::new(DecodeStats::default()),
            interop: OnceLock::new(),
        })
    }

    pub fn project(&self) -> Arc<Project> {
        self.project.read().unwrap().clone()
    }

    /// A new project state (edit, undo, open): cached frames are stale.
    pub fn set_project(&self, p: Arc<Project>) {
        *self.project.write().unwrap() = p;
        self.invalidate();
    }

    /// Media changed (proxy ready, audio loaded): drop cached frames.
    pub fn invalidate(&self) {
        self.ring.lock().unwrap().clear();
        self.wake.notify_all();
    }

    fn has_audio(&self, p: &Project) -> bool {
        let lib = self.lib.read().unwrap();
        let with_audio = |m: &Id| lib.get(m).is_some_and(|e| e.audio.is_some());
        !p.timeline.music.is_empty() || p.timeline.main.iter().any(|c| with_audio(&c.media))
    }

    fn pos(&self, t: &Transport) -> Tick {
        if !t.playing {
            return t.anchor;
        }
        if t.audio_clock {
            return mix::tick_of(self.audio_cursor.load(Ordering::Relaxed));
        }
        let dt = t.anchor_at.elapsed().as_secs_f64() * t.rate;
        t.anchor + Tick::from_seconds_f64(dt)
    }

    /// Clock time, clamped to the timeline; stops playback at either end.
    pub fn position(&self) -> (Tick, bool) {
        let mut t = self.transport.lock().unwrap();
        let end = self.project().duration();
        let p = self.pos(&t);
        if t.playing && (p >= end || p <= Tick::ZERO && t.rate < 0.0) {
            let p = p.clamp(Tick::ZERO, end);
            self.halt(&mut t, p);
            return (p, false);
        }
        (p.clamp(Tick::ZERO, end.max(Tick::ZERO)), t.playing)
    }

    fn halt(&self, t: &mut Transport, at: Tick) {
        t.playing = false;
        t.rate = 0.0;
        t.anchor = at;
        self.audio_running.store(false, Ordering::Relaxed);
    }

    pub fn play(&self, rate: f64) {
        let p = self.project();
        let has_audio = self.has_audio(&p);
        let mut t = self.transport.lock().unwrap();
        let now = self.pos(&t).clamp(Tick::ZERO, p.duration());
        if rate == 0.0 {
            self.halt(&mut t, now);
        } else {
            // Play from the start again when parked at the end.
            let now = if rate > 0.0 && now >= p.duration() {
                Tick::ZERO
            } else {
                now
            };
            t.playing = true;
            t.rate = rate;
            t.anchor = now;
            t.anchor_at = Instant::now();
            t.audio_clock = rate == 1.0 && has_audio && self.audio_live.load(Ordering::Relaxed);
            self.audio_cursor
                .store(mix::sample_of(now), Ordering::Relaxed);
            self.audio_running.store(t.audio_clock, Ordering::Relaxed);
        }
        drop(t);
        self.wake.notify_all();
    }

    pub fn seek(&self, to: Tick) {
        let end = self.project().duration();
        let mut t = self.transport.lock().unwrap();
        let to = to.clamp(Tick::ZERO, end.max(Tick::ZERO));
        t.anchor = to;
        t.anchor_at = Instant::now();
        self.audio_cursor
            .store(mix::sample_of(to), Ordering::Relaxed);
        drop(t);
        self.wake.notify_all();
    }

    pub fn rate(&self) -> f64 {
        self.transport.lock().unwrap().rate
    }

    /// The frame to show now: the ring's frame at the clock (or the closest earlier one in play
    /// order while the decoder catches up). `None` when nothing is decoded yet.
    pub fn display(&self) -> (Tick, Option<(i64, Shown)>) {
        let (t, _) = self.position();
        let p = self.project();
        let f = p.settings.fps.frame_at(t);
        let ring = self.ring.lock().unwrap();
        let hit = ring.get(&f).map(|s| (f, s.clone())).or_else(|| {
            if self.rate() < 0.0 {
                ring.range(f..).next().map(|(k, s)| (*k, s.clone()))
            } else {
                ring.range(..f).next_back().map(|(k, s)| (*k, s.clone()))
            }
        });
        (t, hit)
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.audio_running.store(false, Ordering::Relaxed);
        self.wake.notify_all();
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Next frame to decode: the clock frame, then up to RING-1 ahead in play order.
    fn next_wanted(&self) -> Option<(Arc<Project>, i64)> {
        let (t, playing) = self.position();
        let p = self.project();
        let fps = p.settings.fps;
        let last = fps.frame_at(p.duration() - Tick(1)).max(0);
        let f0 = fps.frame_at(t);
        let rate = self.rate();
        let step = if rate < 0.0 { -1 } else { 1 } * (rate.abs().round() as i64).max(1);
        let ahead = if playing { RING as i64 - 2 } else { 1 };
        let mut ring = self.ring.lock().unwrap();
        // Drop frames the clock has passed.
        ring.retain(|k, _| if step > 0 { *k >= f0 - 1 } else { *k <= f0 + 1 });
        while ring.len() > RING {
            let far = if step > 0 {
                *ring.keys().next_back().unwrap()
            } else {
                *ring.keys().next().unwrap()
            };
            ring.remove(&far);
        }
        (0..ahead)
            .map(|i| f0 + i * step)
            .take_while(|f| (0..=last).contains(f))
            .find(|f| !ring.contains_key(f))
            .map(|f| (p.clone(), f))
    }

    /// Decode-ahead loop; run on its own thread.
    pub fn decode_loop(self: Arc<Self>, tools: Tools) {
        let mut dec = match self.interop.get() {
            Some(i) => Decoders::zero_copy(tools, i.clone()),
            None => Decoders::new(tools, crate::library::decode_device()),
        };
        while !self.stopped() {
            let Some((p, f)) = self.next_wanted() else {
                let ring = self.ring.lock().unwrap();
                let _ = self.wake.wait_timeout(ring, Duration::from_millis(4));
                continue;
            };
            let t = p.settings.fps.tick_of(f);
            let shown = p.layers_at(t).into_iter().find_map(|l| match l {
                Layer::Video {
                    media, source_time, ..
                } => Some((media, source_time)),
                _ => None,
            });
            let started = Instant::now();
            let shown: Shown = shown.and_then(|(media, st)| {
                let e = self.lib.read().unwrap().get(&media).cloned()?;
                let proxy = self.use_proxies.load(Ordering::Relaxed);
                match dec.decode(&e, st, proxy) {
                    Ok(fr) => Some((media, fr)),
                    Err(err) => {
                        eprintln!("decode: {}", err.message);
                        None
                    }
                }
            });
            self.stats
                .lock()
                .unwrap()
                .decode_ms
                .push(started.elapsed().as_secs_f64() * 1e3);
            // Only keep it if nothing changed meanwhile (edit, undo).
            if Arc::ptr_eq(&p, &self.project()) {
                self.ring.lock().unwrap().insert(f, shown);
            }
        }
    }

    /// Silent sink with the device's timing: mixes 10 ms blocks at real-time pace into a scratch
    /// buffer and drops them, so the audio clock and mix cost stay realistic with no sound.
    fn null_sink(&self) {
        self.audio_live.store(true, Ordering::Relaxed);
        let mut buf = vec![0f32; (mix::RATE as usize / 100) * 2];
        let start = Instant::now();
        let mut blocks = 0u64;
        while !self.stopped() {
            if self.audio_running.load(Ordering::Relaxed) {
                let at = self.audio_cursor.load(Ordering::Relaxed);
                if let (Ok(p), Ok(lib)) = (self.project.try_read(), self.lib.try_read()) {
                    mix::mix(&p, &lib, at, &mut buf);
                }
                self.audio_cursor
                    .fetch_add((buf.len() / 2) as i64, Ordering::Relaxed);
            }
            blocks += 1;
            let due = start + Duration::from_millis(10 * blocks);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        self.audio_live.store(false, Ordering::Relaxed);
    }

    /// Audio output on the default device (48 kHz stereo f32; WASAPI converts), or the silent
    /// [`null_sink`](Self::null_sink) when `null` (tests, `--measure`, `EDITELIER_AUDIO=null`).
    /// Returns when the engine stops. Without a device playback runs on the timer clock.
    pub fn audio_loop(self: Arc<Self>, null: bool) {
        if null {
            eprintln!("audio: null sink (silent)");
            return self.null_sink();
        }
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            eprintln!("audio: no output device; using the timer clock");
            return;
        };
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: mix::RATE as u32,
            buffer_size: cpal::BufferSize::Default,
        };
        let (cursor, running, me) = (
            self.audio_cursor.clone(),
            self.audio_running.clone(),
            self.clone(),
        );
        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if !running.load(Ordering::Relaxed) {
                    data.fill(0.0);
                    return;
                }
                let at = cursor.load(Ordering::Relaxed);
                match (me.project.try_read(), me.lib.try_read()) {
                    (Ok(p), Ok(lib)) => mix::mix(&p, &lib, at, data),
                    _ => data.fill(0.0),
                }
                cursor.fetch_add((data.len() / 2) as i64, Ordering::Relaxed);
            },
            |e| eprintln!("audio: {e}"),
            None,
        );
        let stream = match stream.and_then(|s| s.play().map(|()| s)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("audio: {e}; using the timer clock");
                return;
            }
        };
        self.audio_live.store(true, Ordering::Relaxed);
        while !self.stopped() {
            std::thread::sleep(Duration::from_millis(100));
        }
        self.audio_live.store(false, Ordering::Relaxed);
        drop(stream);
    }
}
