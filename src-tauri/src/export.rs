//! Export job: renders every output frame at constant frame rate from the original media (never
//! proxies), mixes the audio once (optional loudness normalise to the preset target), and feeds
//! the Media Foundation exporter (D3, D4, D6).

use crate::api::{ApiError, ApiResult};
use crate::library::{Decoders, MediaEntry, decode_device};
use crate::mix;
use editelier_core::{FrameRate, Id, Layer, Project, Tick, TitleClip};
use editelier_export::win::{ExportOptions, Exporter};
use editelier_export::{CancelToken, ExportError, Preset, loudness};
use editelier_media::Tools;
use editelier_render::{Fonts, Gpu, Renderer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// UI preset id (`ui/src/model.ts` PRESETS) -> the built-in preset for this project.
pub fn preset_for(ui_id: &str, p: &Project) -> ApiResult<Preset> {
    let fps = p.settings.fps;
    let high = fps.as_f64() > 31.0;
    let id = match ui_id {
        "youtube-1080" if high => "youtube-1080p60",
        "youtube-1080" => "youtube-1080p30",
        "youtube-4k" if high => "youtube-4k60",
        "youtube-4k" => "youtube-4k30",
        "tiktok" => "tiktok-1080x1920",
        "instagram-reel" => "instagram-reel-1080x1920",
        "instagram-45" => "instagram-feed-1080x1350",
        "master" if p.settings.width.max(p.settings.height) > 1920 => "master-4k",
        "master" => "master-1080p",
        other => other,
    };
    let mut preset = Preset::builtin_by_id(id)
        .ok_or_else(|| ApiError::new("invalid", "Unknown export preset."))?;
    if ui_id == "master" {
        (preset.width, preset.height) = (p.settings.width, p.settings.height);
    }
    // The timeline's frame rate wins; the preset sets size, codec and bitrate.
    (preset.fps_num, preset.fps_den) = (fps.num as u32, fps.den as u32);
    preset
        .validate()
        .map_err(|e| ApiError::new("invalid", e.to_string()))?;
    Ok(preset)
}

pub struct Job {
    pub project: Arc<Project>,
    pub media: HashMap<Id, Arc<MediaEntry>>,
    pub preset: Preset,
    pub path: PathBuf,
    /// Timeline range, frame-snapped by the caller.
    pub range: (Tick, Tick),
    pub cancel: CancelToken,
}

pub struct Done {
    pub frames: u64,
    pub seconds: f64,
    pub encoder: String,
}

fn export_err(e: ExportError) -> ApiError {
    match e {
        ExportError::Cancelled => ApiError::new("cancelled", "Export cancelled."),
        ExportError::Io(io) if io.raw_os_error() == Some(112) => {
            ApiError::new("disk_full", "The disk is full.")
        }
        e => ApiError::new("export_failed", e.to_string()),
    }
}

/// First sample of output frame `n` (exact, rounds down).
fn frame_sample(fps: FrameRate, n: i64) -> i64 {
    (n as i128 * mix::RATE as i128 * fps.den as i128 / fps.num as i128) as i64
}

/// Runs the whole export on the calling thread. `progress(0..=1)` after each frame.
pub fn run(job: Job, tools: Tools, gpu: &Gpu, mut progress: impl FnMut(f32)) -> ApiResult<Done> {
    let started = Instant::now();
    let p = &job.project;
    let fps = p.settings.fps;
    let (f0, f1) = (fps.frame_at(job.range.0), fps.frame_at(job.range.1));
    let total = (f1 - f0).max(0);
    if total == 0 {
        return Err(ApiError::new(
            "empty_timeline",
            "Add a clip to the timeline before exporting.",
        ));
    }
    let preset = &job.preset;
    let size = (preset.width, preset.height);

    // Audio for the whole range, mixed once so loudness can be measured first.
    let audio = if preset.audio.is_some() {
        let s0 = mix::sample_of(fps.tick_of(f0));
        let n = frame_sample(fps, total) as usize;
        let mut buf = vec![0f32; n * 2];
        mix::mix(p, &job.media, s0, &mut buf);
        if let Some(target) = preset.loudness {
            let r = loudness::measure(&buf, target).map_err(export_err)?;
            // ponytail: no true-peak limiter; lower the gain to respect the ceiling instead.
            let gain = if r.needs_limiter {
                r.gain_db.min(target.true_peak_dbtp - r.true_peak_dbtp)
            } else {
                r.gain_db
            };
            if gain.is_finite() && gain != 0.0 {
                let g = loudness::db_to_linear(gain);
                buf.iter_mut().for_each(|s| *s *= g);
            }
        }
        Some(buf)
    } else {
        None
    };

    let mut exporter = Exporter::new(
        &job.path,
        preset,
        ExportOptions {
            total_frames: Some(total as u64),
            cancel: job.cancel.clone(),
            ..Default::default()
        },
    )
    .map_err(export_err)?;
    let encoder = exporter.encoder().0.to_owned();
    let mut renderer = Renderer::new(gpu, Fonts::bundled());
    let mut dec = Decoders::new(tools, decode_device());
    let mut last: Option<(Id, (i64, bool))> = None;
    let title_scale = preset.height as f32 / p.settings.height.max(1) as f32;
    for n in 0..total {
        if job.cancel.is_cancelled() {
            return Err(export_err(ExportError::Cancelled));
        }
        let layers = p.layers_at(fps.tick_of(f0 + n));
        let mut titles: Vec<&TitleClip> = Vec::new();
        let mut video = None;
        for l in &layers {
            match l {
                Layer::Video {
                    media, source_time, ..
                } => video = Some((*media, *source_time)),
                Layer::Title(t) => titles.push(t),
                Layer::Music { .. } => {}
            }
        }
        match video.and_then(|(m, st)| Some((m, st, job.media.get(&m)?))) {
            Some((m, st, e)) => {
                let f = dec.decode(e, st, false)?;
                if last != Some((m, f.key)) {
                    renderer
                        .upload(&f.source())
                        .map_err(|e| ApiError::new("export_failed", e.to_string()))?;
                    last = Some((m, f.key));
                }
            }
            None => {
                renderer.clear_clip();
                last = None;
            }
        }
        renderer.compose(size, &titles, title_scale);
        let nv12 = renderer
            .read_nv12()
            .map_err(|e| ApiError::new("export_failed", e.to_string()))?;
        exporter.push_video_nv12(&nv12).map_err(export_err)?;
        if let Some(a) = &audio {
            let (s0, s1) = (
                frame_sample(fps, n) as usize,
                frame_sample(fps, n + 1) as usize,
            );
            if s1 > s0 {
                exporter
                    .push_audio(&a[s0 * 2..s1 * 2])
                    .map_err(export_err)?;
            }
        }
        progress((n + 1) as f32 / total as f32);
    }
    let stats = exporter.finish().map_err(export_err)?;
    Ok(Done {
        frames: stats.frames,
        seconds: started.elapsed().as_secs_f64(),
        encoder,
    })
}

#[cfg(test)]
mod tests {
    //! End to end: generated media (`scripts/make-test-media.ps1`) → project built through the
    //! command layer → 1080p export on the hardware encoder → checked with the system ffprobe /
    //! ffmpeg (test oracle). Skips when media, FFmpeg helper, GPU or oracle is missing.
    use super::*;
    use crate::commands::{Core, Dirs};
    use editelier_core::{FrameRate, Session, Settings};
    use serde_json::{Value, json};
    use std::path::Path;
    use std::process::Command;

    fn oracle(args: &[&str], input: &Path, tail: &[&str]) -> Option<Vec<u8>> {
        let out = Command::new("ffmpeg")
            .args(["-v", "error"])
            .args(args)
            .arg("-i")
            .arg(input)
            .args(tail)
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// Frame `n` (presentation order) of `path` as 1920x1080 grey.
    fn grey_frame(path: &Path, n: usize) -> Vec<u8> {
        let vf = format!("select=eq(n\\,{n}),scale=1920:1080:flags=bilinear");
        oracle(
            &[],
            path,
            &[
                "-vf",
                &vf,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "-",
            ],
        )
        .expect("oracle frame")
    }

    #[test]
    fn export_end_to_end_1080p() {
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-media");
        let (a, b, tone) = (
            media.join("cfr30_1080p_h264_aac.mp4"),
            media.join("ntsc2997_720p_h264.mp4"),
            media.join("tone_48k.wav"),
        );
        if ![&a, &b, &tone].iter().all(|p| p.is_file()) {
            eprintln!("SKIP: run scripts/make-test-media.ps1");
            return;
        }
        if oracle(&["-version"], Path::new("x"), &[]).is_none()
            && Command::new("ffprobe").arg("-version").output().is_err()
        {
            eprintln!("SKIP: no system ffmpeg/ffprobe oracle");
            return;
        }
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("SKIP: no GPU");
            return;
        };
        let tmp = std::env::temp_dir().join("editelier-e2e");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let core = Core::new(
            Dirs {
                cache: tmp.join("cache"),
                data: tmp.join("data"),
                videos: None,
                documents: None,
            },
            Some(gpu.clone()),
        );
        let Some(tools) = core.tools.clone() else {
            eprintln!("SKIP: FFmpeg helper missing (scripts/fetch-ffmpeg.ps1)");
            return;
        };
        let fps = FrameRate::FPS_30;
        core.replace_doc(
            Session::new(Project::new(Settings {
                fps,
                ..Settings::default()
            })),
            None,
        );
        let paths: Vec<String> = [&a, &b, &tone]
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        core.import(None, &paths).unwrap();
        let f = |n: i64| fps.tick_of(n).0;
        let run = |name: &str, args: Value| core.run_command(name, &args).unwrap();
        let ids = |s: &Value, k: &str| -> Vec<String> {
            s["project"]["timeline"][k]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_owned())
                .collect()
        };
        // Media ids 1, 2, 3 = a, b, tone. Main track: a, b, a.
        run("timeline.append", json!({"media_id": "1"}));
        run("timeline.append", json!({"media_id": "2"}));
        let s = serde_json::to_value(run("timeline.append", json!({"media_id": "1"}))).unwrap();
        let main = ids(&s, "main");
        // Split the first clip at frame 30, move the right half's in point to 60.
        run("clip.split", json!({"at": f(30)}));
        let right = core.engine.project().timeline.main[1].id.to_string();
        run(
            "clip.trim",
            json!({"clip_id": right, "edge": "in", "delta": f(30)}),
        );
        // b keeps 45 frames, the last a keeps 30.
        let out_of = |i: usize| core.engine.project().timeline.main[i].source_out.0;
        run(
            "clip.trim",
            json!({"clip_id": main[1], "edge": "out", "delta": f(45) - out_of(2)}),
        );
        run(
            "clip.trim",
            json!({"clip_id": main[2], "edge": "out", "delta": f(30) - out_of(3)}),
        );
        run(
            "title.add",
            json!({"start": f(15), "duration": f(30), "text": "Editelier"}),
        );
        let s =
            serde_json::to_value(run("music.add", json!({"media_id": "3", "start": 0}))).unwrap();
        let music = ids(&s, "music");
        run(
            "music.update",
            json!({"id": music[0], "gain_db": -3, "fade_in": f(15), "fade_out": f(15)}),
        );

        let project = core.engine.project();
        // Expected main track: a[0..30) a[60..120) b[0..45) a[0..30) = 165 frames.
        let ins: Vec<i64> = project
            .timeline
            .main
            .iter()
            .map(|c| fps.frame_at(c.source_in))
            .collect();
        let outs: Vec<i64> = project
            .timeline
            .main
            .iter()
            .map(|c| fps.frame_at(c.source_out))
            .collect();
        assert_eq!((ins, outs), (vec![0, 60, 0, 0], vec![30, 120, 45, 30]));
        let out = tmp.join("e2e.mp4");
        let preset = preset_for("youtube-1080", &project).unwrap();
        let end = fps.tick_of(165);
        let job = Job {
            media: core.lib.read().unwrap().clone(),
            project: project.clone(),
            preset,
            path: out.clone(),
            range: (Tick::ZERO, end),
            cancel: CancelToken::default(),
        };
        let done = run_job(job, tools, &gpu);
        eprintln!(
            "e2e export: {} frames in {:.2} s = {:.2}x realtime on {}",
            done.frames,
            done.seconds,
            5.5 / done.seconds,
            done.encoder
        );
        assert_eq!(done.frames, 165);

        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
            .args([
                "-show_entries",
                "stream=nb_read_frames,width,height:format=duration",
            ])
            .args(["-of", "json"])
            .arg(&out)
            .output()
            .unwrap();
        let j: Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(j["streams"][0]["nb_read_frames"], "165");
        assert_eq!(j["streams"][0]["width"], 1920);
        let dur: f64 = j["format"]["duration"].as_str().unwrap().parse().unwrap();
        assert!((dur - 5.5).abs() < 0.05, "duration {dur}");

        // Output frame N shows the expected source frame (burned frame numbers), and matches it
        // better than its neighbours.
        let check = |n: usize, src: &Path, want: usize| {
            let got = grey_frame(&out, n);
            let score = |k: usize| editelier_render::ssim(&got, &grey_frame(src, k), 1920, 1080);
            let best = score(want);
            eprintln!("frame {n}: SSIM {best:.4} vs source frame {want}");
            assert!(best >= 0.97, "frame {n}: SSIM {best}");
            for k in [want.saturating_sub(1), want + 1] {
                if k != want {
                    assert!(score(k) < best, "frame {n} looks more like source {k}");
                }
            }
        };
        check(5, &a, 5);
        check(75, &a, 105);
        // b is 29.97 fps on a 30 fps timeline: timeline 10/30 s holds b's frame 9 (PTS 0.3003 s).
        check(100, &b, 9);
        check(150, &a, 15);
    }

    fn run_job(job: Job, tools: Tools, gpu: &Gpu) -> Done {
        // One Media Foundation encoder session at a time in this crate's tests.
        static MF: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = MF.lock().unwrap_or_else(|e| e.into_inner());
        run(job, tools, gpu, |_| {}).unwrap()
    }
}
