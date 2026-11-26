//! Export job: renders every output frame at constant frame rate from the original media (never
//! proxies), mixes the audio once (optional loudness normalise to the preset target), and feeds
//! the Media Foundation exporter (D3, D4, D6). Decode, render and encode are pipelined stages;
//! frames stay on the GPU end to end where the hardware allows (A-PERF-03/04).

use crate::api::{ApiError, ApiResult};
use crate::library::{Decoders, MediaEntry, OwnedFrame, decode_device};
use crate::mix;
use editelier_core::{FrameRate, Id, Layer, Project, Tick, TitleClip};
use editelier_export::win::{
    ExportOptions, ExportStats, Exporter, device_manager_for, enumerate_encoders,
};
use editelier_export::{CancelToken, ExportError, Preset, loudness};
use editelier_media::Tools;
use editelier_render::{Fonts, Gpu, Interop, Nv12Frame, Renderer};
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

/// Audio for the whole range, mixed once so loudness can be measured first.
fn mix_audio(job: &Job, f0: i64, total: i64) -> ApiResult<Option<Vec<f32>>> {
    let (p, preset) = (&job.project, &job.preset);
    if preset.audio.is_none() {
        return Ok(None);
    }
    let fps = p.settings.fps;
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
    Ok(Some(buf))
}

/// A rendered frame on its way to the encoder.
enum Out {
    Gpu(Nv12Frame),
    Cpu(Vec<u8>),
}

/// Frames queued between pipeline stages (each queue holds this many).
const QUEUE: usize = 3;

fn render_err(e: editelier_render::RenderError) -> ApiError {
    ApiError::new("export_failed", e.to_string())
}

fn joined<T>(h: std::thread::ScopedJoinHandle<'_, ApiResult<T>>) -> ApiResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(ApiError::new("export_failed", "Export worker crashed.")))
}

/// Runs the whole export: decode-ahead, render (calling thread), encode and the audio mix run
/// as overlapping stages with bounded queues. Decoded frames and NV12 output stay on the GPU
/// when [`Interop`] works on this adapter and the encoder is a hardware MFT on the same one;
/// otherwise frames go through the CPU. `progress(0..=1)` after each rendered frame.
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
    let encoder = enumerate_encoders(preset.video.codec)
        .map_err(export_err)?
        .into_iter()
        .next()
        .ok_or_else(|| export_err(ExportError::Unsupported("no video encoder".into())))?;
    let interop = Interop::new(gpu)
        .map_err(|e| eprintln!("export: zero-copy off ({e})"))
        .ok();
    // GPU output needs the encoder on the shared device's adapter.
    let gpu_out = interop
        .as_ref()
        .filter(|i| encoder.hardware && encoder.adapter_luid.is_none_or(|l| l == i.adapter_luid()));
    let shared_device = gpu_out.map(|i| i.device().clone());
    let title_scale = preset.height as f32 / p.settings.height.max(1) as f32;
    let cancel = &job.cancel;
    let job = &job;

    std::thread::scope(|s| {
        let audio = s.spawn(|| mix_audio(job, f0, total));
        let (vtx, vrx) = std::sync::mpsc::sync_channel::<Option<(Id, Arc<OwnedFrame>)>>(QUEUE);
        let dec_interop = interop.clone();
        let decode = s.spawn(move || -> ApiResult<f64> {
            let mut busy = 0.0;
            let mut dec = match dec_interop {
                Some(i) => Decoders::zero_copy(tools, i),
                None => Decoders::new(tools, decode_device()),
            };
            for n in 0..total {
                let video = job
                    .project
                    .layers_at(fps.tick_of(f0 + n))
                    .into_iter()
                    .find_map(|l| match l {
                        Layer::Video {
                            media, source_time, ..
                        } => Some((media, source_time)),
                        _ => None,
                    });
                let t = Instant::now();
                let pic = match video.and_then(|(m, st)| Some((m, st, job.media.get(&m)?))) {
                    Some((m, st, e)) => Some((m, dec.decode(e, st, false)?)),
                    None => None,
                };
                busy += t.elapsed().as_secs_f64();
                if vtx.send(pic).is_err() {
                    break; // render stopped
                }
            }
            Ok(busy)
        });
        let (etx, erx) = std::sync::mpsc::sync_channel::<Out>(QUEUE);
        let encode = s.spawn(move || -> ApiResult<(ExportStats, f64)> {
            let mut exporter = Exporter::new(
                &job.path,
                &job.preset,
                ExportOptions {
                    encoder: Some(encoder),
                    // Built here: MF device managers aren't `Send`.
                    device_manager: match &shared_device {
                        Some(d) => Some(device_manager_for(d).map_err(export_err)?),
                        None => None,
                    },
                    total_frames: Some(total as u64),
                    cancel: job.cancel.clone(),
                    ..Default::default()
                },
            )
            .map_err(export_err)?;
            let audio = joined(audio)?;
            let (mut n, mut busy) = (0, 0.0);
            for out in erx {
                let t = Instant::now();
                match out {
                    Out::Gpu(f) => exporter.push_video_copy(|dst, sub| {
                        f.interop()
                            .copy_out(&f, dst, sub)
                            .map_err(|e| ExportError::Media(e.0))
                    }),
                    Out::Cpu(nv12) => exporter.push_video_nv12(&nv12),
                }
                .map_err(export_err)?;
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
                busy += t.elapsed().as_secs_f64();
                n += 1;
            }
            if n != total {
                // Render stopped early: dropping the exporter deletes the partial file.
                return Err(export_err(ExportError::Cancelled));
            }
            Ok((exporter.finish().map_err(export_err)?, busy))
        });

        let rendered = (|| -> ApiResult<f64> {
            let mut busy = 0.0;
            let mut renderer = Renderer::new(gpu, Fonts::bundled());
            let mut last: Option<(Id, (i64, bool))> = None;
            for n in 0..total {
                if cancel.is_cancelled() {
                    return Err(export_err(ExportError::Cancelled));
                }
                let Ok(pic) = vrx.recv() else {
                    return Ok(busy); // decode failed; its error wins
                };
                let t = Instant::now();
                match pic {
                    Some((m, f)) => {
                        if last != Some((m, f.key)) {
                            renderer.upload(&f.source()).map_err(render_err)?;
                            last = Some((m, f.key));
                        }
                    }
                    None => {
                        renderer.clear_clip();
                        last = None;
                    }
                }
                let layers = job.project.layers_at(fps.tick_of(f0 + n));
                let titles: Vec<&TitleClip> = layers
                    .iter()
                    .filter_map(|l| match l {
                        Layer::Title(t) => Some(*t),
                        _ => None,
                    })
                    .collect();
                renderer.compose(size, &titles, title_scale);
                let out = match gpu_out {
                    Some(i) => Out::Gpu(renderer.render_nv12(i).map_err(render_err)?),
                    None => Out::Cpu(renderer.read_nv12().map_err(render_err)?),
                };
                busy += t.elapsed().as_secs_f64();
                if etx.send(out).is_err() {
                    return Ok(busy); // encoder failed; its error wins
                }
                progress((n + 1) as f32 / total as f32);
            }
            Ok(busy)
        })();
        drop((vrx, etx));
        let decoded = joined(decode);
        let stats = joined(encode);
        let rendered = rendered?;
        let decoded = decoded?;
        let (stats, encoded) = stats?;
        let seconds = started.elapsed().as_secs_f64();
        eprintln!(
            "export: {total} frames in {seconds:.1} s, zero-copy {}; busy s: decode {decoded:.1}, \
             render {rendered:.1} (of which waiting on D3D11 {:.1}), encode {encoded:.1}",
            gpu_out.is_some(),
            interop.as_ref().map_or(0.0, |i| i.waited_seconds()),
        );
        Ok(Done {
            frames: stats.frames,
            seconds,
            encoder: stats.encoder_name,
        })
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

    /// A-PERF-03/04 bench: a 20 s timeline of `test-media/bench20_{1080p,2160p}30.mp4` (one clip,
    /// a title, its own audio) exported with the YouTube preset. `cargo test --release -p
    /// editelier -- --ignored export_bench --test-threads=1 --nocapture`.
    #[test]
    #[ignore = "perf bench; needs the bench20 clips (see docs/next-session.md)"]
    fn export_bench() {
        for (name, ui, (w, h)) in [
            ("bench20_1080p30.mp4", "youtube-1080", (1920, 1080)),
            ("bench20_2160p30.mp4", "youtube-4k", (3840, 2160)),
        ] {
            let src = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test-media")
                .join(name);
            if !src.is_file() {
                eprintln!("SKIP: {name} missing");
                continue;
            }
            let Ok(gpu) = Gpu::headless() else {
                eprintln!("SKIP: no GPU");
                return;
            };
            let tmp = std::env::temp_dir().join("editelier-bench");
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
                eprintln!("SKIP: FFmpeg helper missing");
                return;
            };
            let fps = FrameRate::FPS_30;
            core.replace_doc(
                Session::new(Project::new(Settings {
                    fps,
                    width: w,
                    height: h,
                    ..Settings::default()
                })),
                None,
            );
            core.import(None, &[src.display().to_string()]).unwrap();
            core.run_command("timeline.append", &json!({"media_id": "1"}))
                .unwrap();
            core.run_command(
                "title.add",
                &json!({"start": 0, "duration": fps.tick_of(300).0, "text": "Editelier"}),
            )
            .unwrap();
            let project = core.engine.project();
            let job = Job {
                media: core.lib.read().unwrap().clone(),
                project: project.clone(),
                preset: preset_for(ui, &project).unwrap(),
                path: tmp.join("bench.mp4"),
                range: (Tick::ZERO, fps.tick_of(600)),
                cancel: CancelToken::default(),
            };
            let done = run_job(job, tools, &gpu);
            eprintln!(
                "BENCH {name}: {} frames in {:.2} s = {:.2}x realtime on {}",
                done.frames,
                done.seconds,
                20.0 / done.seconds,
                done.encoder
            );
            assert_eq!(done.frames, 600);
        }
    }

    /// Preview throughput bench (headless): decode + upload + compose + output transform for
    /// 300 sequential frames into an offscreen 1920x1200 target, zero-copy unless
    /// `EDITELIER_ZERO_COPY=0`. Same invocation as [`export_bench`].
    #[test]
    #[ignore = "perf bench; needs the bench20 clips"]
    fn preview_bench() {
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("SKIP: no GPU");
            return;
        };
        let tmp = std::env::temp_dir().join("editelier-preview-bench");
        let _ = std::fs::remove_dir_all(&tmp);
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
            eprintln!("SKIP: FFmpeg helper missing");
            return;
        };
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-media");
        let clips = [
            ("1080p original", "bench20_1080p30.mp4", false),
            ("4K original", "bench20_2160p30.mp4", false),
            ("4K via proxy", "bench20_2160p30.mp4", true),
        ];
        if !clips.iter().all(|c| media.join(c.1).is_file()) {
            eprintln!("SKIP: bench20 clips missing");
            return;
        }
        core.replace_doc(Session::new(Project::new(Settings::default())), None);
        let paths: Vec<String> = ["bench20_1080p30.mp4", "bench20_2160p30.mp4"]
            .iter()
            .map(|n| media.join(n).display().to_string())
            .collect();
        core.import(None, &paths).unwrap();
        let interop = Interop::new(&gpu).ok();
        let started = Instant::now();
        for (label, name, proxy) in clips {
            let id = if name.contains("1080") { "1" } else { "2" };
            let e = loop {
                let e = core.lib.read().unwrap()[&id.parse().unwrap()].clone();
                let ready = matches!(e.proxy, crate::library::Proxy::Ready { .. });
                if !proxy || ready || started.elapsed().as_secs() > 300 {
                    break e;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            };
            let mut dec = match &interop {
                Some(i) => Decoders::zero_copy(tools.clone(), i.clone()),
                None => Decoders::new(tools.clone(), decode_device()),
            };
            let mut renderer = Renderer::new(&gpu, Fonts::bundled());
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1920,
                    height: 1200,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let fps = FrameRate::FPS_30;
            let t0 = Instant::now();
            let n = 300;
            for i in 0..n {
                let f = dec.decode(&e, fps.tick_of(i), proxy).unwrap();
                renderer.upload(&f.source()).unwrap();
                renderer.compose((1920, 1080), &[], 1.0);
                let mut enc = gpu.device.create_command_encoder(&Default::default());
                renderer.encode_output(&mut enc, &view, format, (1920, 1200));
                gpu.queue.submit([enc.finish()]);
                // Like a presented frame: don't let the CPU run more than a frame ahead.
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
            }
            let fps = n as f64 / t0.elapsed().as_secs_f64();
            eprintln!(
                "PREVIEW {label} (zero-copy {}): {fps:.1} fps",
                interop.is_some()
            );
        }
    }

    fn run_job(job: Job, tools: Tools, gpu: &Gpu) -> Done {
        // One Media Foundation encoder session at a time in this crate's tests.
        static MF: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = MF.lock().unwrap_or_else(|e| e.into_inner());
        run(job, tools, gpu, |_| {}).unwrap()
    }
}
