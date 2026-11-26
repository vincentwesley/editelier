//! Media Foundation export against the system ffprobe/ffmpeg oracle (test-only; never shipped).
//! Conformance runs by default (skips without ffprobe). Quality + speed (VMAF/SSIM,
//! realtime factor, A-QUAL-01/02, A-PERF-03/04) is milestone-only:
//! `cargo test -p editelier-export --release -- --ignored --nocapture`.
#![cfg(windows)]

use editelier_export::win::{ExportOptions, Exporter, enumerate_encoders, manager_device};
use editelier_export::{CancelToken, EncoderInfo, ExportError, Preset, VideoCodec};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Tests that open Media Foundation encoder sessions run one at a time: parallel hardware
/// sessions (QSV) fail intermittently.
fn mf_lock() -> std::sync::MutexGuard<'static, ()> {
    static MF: std::sync::Mutex<()> = std::sync::Mutex::new(());
    MF.lock().unwrap_or_else(|e| e.into_inner())
}

fn tool(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("editelier-export-tests")
        .join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Moving diagonal gradient + chroma ramps + a 16-bit binary frame counter (top-left).
fn pattern(w: usize, h: usize, frame: u64) -> Vec<u8> {
    let mut f = vec![0u8; w * h * 3 / 2];
    let (y, uv) = f.split_at_mut(w * h);
    let shift = frame as usize * 6;
    for row in 0..h {
        for (x, px) in y[row * w..(row + 1) * w].iter_mut().enumerate() {
            *px = (16 + ((x + row / 2 + shift) % 440).abs_diff(220) * 219 / 220) as u8;
        }
    }
    let block = (w / 40).max(8);
    for bit in 0..16 {
        let on = (frame >> bit) & 1 == 1;
        for row in block..2 * block {
            let x0 = block + bit * block * 3 / 2;
            y[row * w + x0..row * w + x0 + block].fill(if on { 235 } else { 16 });
        }
    }
    for row in 0..h / 2 {
        for cx in 0..w / 2 {
            let o = row * w + cx * 2;
            uv[o] = 128 + ((cx + shift / 2) % 96) as u8 - 48;
            uv[o + 1] = 128 + (row % 96) as u8 - 48;
        }
    }
    f
}

fn audio_frame(preset: &Preset, n: u64) -> Vec<f32> {
    let s0 = (preset.frame_time_hns(n) as u128 * 48_000 / 10_000_000) as u64;
    let s1 = (preset.frame_time_hns(n + 1) as u128 * 48_000 / 10_000_000) as u64;
    (s0..s1)
        .flat_map(|i| {
            let v = 0.1 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
            [v, v]
        })
        .collect()
}

/// Encode `frames` of the pattern; returns wall time spent inside the exporter.
fn encode(path: &Path, preset: &Preset, enc: &EncoderInfo, frames: u64) -> (Duration, String) {
    let mut ex = Exporter::new(
        path,
        preset,
        ExportOptions {
            encoder: Some(enc.clone()),
            total_frames: Some(frames),
            ..Default::default()
        },
    )
    .unwrap();
    let mut spent = Duration::ZERO;
    for n in 0..frames {
        let frame = pattern(preset.width as usize, preset.height as usize, n);
        let audio = preset.audio.as_ref().map(|_| audio_frame(preset, n));
        let t = Instant::now();
        ex.push_video_nv12(&frame).unwrap();
        if let Some(a) = audio {
            ex.push_audio(&a).unwrap();
        }
        spent += t.elapsed();
    }
    let t = Instant::now();
    let stats = ex.finish().unwrap();
    spent += t.elapsed();
    assert_eq!(stats.frames, frames);
    (
        spent,
        format!("{} ({:?})", stats.encoder_name, stats.encoder_vendor),
    )
}

fn ffprobe(path: &Path, args: &[&str]) -> Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-of", "json"])
        .args(args)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Top-level box order + edit-list presence, read straight from the file.
fn mp4_layout(path: &Path) -> (Vec<String>, bool) {
    let data = std::fs::read(path).unwrap();
    let (mut boxes, mut elst, mut i) = (Vec::new(), false, 0usize);
    while i + 8 <= data.len() {
        let mut size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        let kind = String::from_utf8_lossy(&data[i + 4..i + 8]).to_string();
        if size == 1 {
            size = u64::from_be_bytes(data[i + 8..i + 16].try_into().unwrap()) as usize;
        } else if size == 0 {
            size = data.len() - i;
        }
        if kind == "moov" {
            elst = data[i..i + size].windows(4).any(|w| w == b"elst");
        }
        boxes.push(kind);
        i += size.max(8);
    }
    (boxes, elst)
}

/// A-QUAL-07 checks. Returns problems instead of panicking so every encoder reports.
fn conformance(path: &Path, preset: &Preset, frames: u64) -> Vec<String> {
    let mut bad = Vec::new();
    let mut check = |ok: bool, what: String| {
        if !ok {
            bad.push(what);
        }
    };
    let probe = ffprobe(path, &["-count_frames", "-show_streams", "-show_format"]);
    let streams = probe["streams"].as_array().unwrap();
    let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let fps = format!("{}/{}", preset.fps_num, preset.fps_den);
    check(
        v["codec_name"] == "h264",
        format!("codec {}", v["codec_name"]),
    );
    check(v["profile"] == "High", format!("profile {}", v["profile"]));
    check(v["width"] == preset.width, format!("width {}", v["width"]));
    check(
        v["height"] == preset.height,
        format!("height {}", v["height"]),
    );
    check(
        v["r_frame_rate"] == fps.as_str(),
        format!("fps {}", v["r_frame_rate"]),
    );
    check(
        v["nb_read_frames"] == frames.to_string().as_str(),
        format!("frames {}", v["nb_read_frames"]),
    );
    let start: f64 = v["start_time"].as_str().unwrap_or("nan").parse().unwrap();
    check(start.abs() < 1e-3, format!("start_time {start}"));
    let dur: f64 = v["duration"].as_str().unwrap_or("nan").parse().unwrap();
    let want = preset.frame_time_hns(frames) as f64 / 1e7;
    check(
        (dur - want).abs() < 0.02,
        format!("duration {dur} want {want}"),
    );
    for (k, want) in [
        ("color_primaries", "bt709"),
        ("color_transfer", "bt709"),
        ("color_space", "bt709"),
        ("color_range", "tv"),
    ] {
        check(v[k] == want, format!("{k} {}", v[k]));
    }
    if preset.audio.is_some() {
        match streams.iter().find(|s| s["codec_type"] == "audio") {
            Some(a) => {
                check(
                    a["codec_name"] == "aac",
                    format!("audio {}", a["codec_name"]),
                );
                check(
                    a["profile"] == "LC",
                    format!("aac profile {}", a["profile"]),
                );
                check(
                    a["sample_rate"] == "48000",
                    format!("rate {}", a["sample_rate"]),
                );
                check(a["channels"] == 2, format!("channels {}", a["channels"]));
            }
            None => check(false, "no audio stream".into()),
        }
    }
    let (boxes, elst) = mp4_layout(path);
    let pos = |k: &str| boxes.iter().position(|b| b == k);
    check(
        pos("moov") < pos("mdat"),
        format!("moov after mdat: {boxes:?}"),
    );
    check(!elst, "edit list (elst) present".into());
    // Keyframes (sync samples = IDR in MF output, so GOPs are closed) at most 2 s apart.
    let pk = ffprobe(
        path,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,flags",
        ],
    );
    let keys: Vec<f64> = pk["packets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["flags"].as_str().unwrap_or("").starts_with('K'))
        .map(|p| p["pts_time"].as_str().unwrap().parse().unwrap())
        .collect();
    let mut gaps: Vec<f64> = keys.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.push(want - keys.last().copied().unwrap_or(0.0));
    let max_gap = gaps.iter().copied().fold(0.0, f64::max);
    check(max_gap <= 2.0 + 1e-6, format!("keyframe gap {max_gap:.2}s"));
    bad
}

fn h264_encoders() -> Vec<EncoderInfo> {
    let list = enumerate_encoders(VideoCodec::H264).unwrap();
    println!("H.264 encoders: {list:#?}");
    assert!(
        list.iter().any(|e| !e.hardware),
        "Microsoft software H.264 MFT missing"
    );
    list
}

#[test]
fn enumerates_and_reports_hevc() {
    let _mf = mf_lock();
    let h264 = h264_encoders();
    assert!(h264.windows(2).all(|w| w[0].vendor <= w[1].vendor));
    let hevc = enumerate_encoders(VideoCodec::Hevc).unwrap();
    println!("HEVC encoders: {hevc:#?}");
}

#[test]
fn every_encoder_writes_conformant_mp4() {
    let _mf = mf_lock();
    if !tool("ffprobe") {
        eprintln!("SKIP: ffprobe not on PATH");
        return;
    }
    let preset = Preset::builtin_by_id("youtube-1080p30").unwrap();
    let dir = out_dir("conformance");
    let mut failures = Vec::new();
    for (i, enc) in h264_encoders().iter().enumerate() {
        let path = dir.join(format!("enc{i}.mp4"));
        let (_, used) = encode(&path, &preset, enc, 30);
        let bad = conformance(&path, &preset, 30);
        println!("{} -> {used}: {bad:?}", enc.name);
        if !bad.is_empty() {
            failures.push(format!("{}: {bad:?}", enc.name));
        }
        let _ = std::fs::remove_file(&path);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn hevc_encodes_when_present() {
    let _mf = mf_lock();
    let Some(enc) = enumerate_encoders(VideoCodec::Hevc)
        .unwrap()
        .into_iter()
        .next()
    else {
        eprintln!("SKIP: no HEVC encoder (Store HEVC Video Extensions not installed?)");
        return;
    };
    let mut preset = Preset::builtin_by_id("youtube-1080p30").unwrap();
    preset.video.codec = VideoCodec::Hevc;
    let path = out_dir("hevc").join("hevc.mp4");
    encode(&path, &preset, &enc, 15);
    if tool("ffprobe") {
        let p = ffprobe(&path, &["-show_streams"]);
        assert_eq!(p["streams"][0]["codec_name"], "hevc");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn zero_copy_texture_input() {
    let _mf = mf_lock();
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    let Some(enc) = enumerate_encoders(VideoCodec::H264)
        .unwrap()
        .into_iter()
        .find(|e| e.hardware)
    else {
        eprintln!("SKIP: no hardware H.264 encoder");
        return;
    };
    let mut preset = Preset::builtin_by_id("youtube-1080p30").unwrap();
    preset.audio = None;
    let path = out_dir("texture").join("texture.mp4");
    let mut ex = Exporter::new(
        &path,
        &preset,
        ExportOptions {
            encoder: Some(enc),
            ..Default::default()
        },
    )
    .unwrap();
    let device = manager_device(ex.device_manager().unwrap()).unwrap();
    let (w, h) = (preset.width, preset.height);
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    for n in 0..30 {
        let frame = pattern(w as usize, h as usize, n);
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: frame.as_ptr() as _,
            SysMemPitch: w,
            SysMemSlicePitch: 0,
        };
        let mut tex = None;
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut tex)) }.unwrap();
        ex.push_video_texture(&tex.unwrap(), 0).unwrap();
    }
    ex.finish().unwrap();
    if tool("ffprobe") {
        let bad = conformance(&path, &preset, 30);
        assert!(bad.is_empty(), "{bad:?}");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cancel_and_error_remove_partial_file() {
    let _mf = mf_lock();
    let preset = Preset::builtin_by_id("tiktok-1080x1920").unwrap();
    let dir = out_dir("cancel");
    let cancel = CancelToken::default();
    let path = dir.join("cancelled.mp4");
    let mut ex = Exporter::new(
        &path,
        &preset,
        ExportOptions {
            cancel: cancel.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let frame = pattern(preset.width as usize, preset.height as usize, 0);
    for _ in 0..5 {
        ex.push_video_nv12(&frame).unwrap();
        ex.push_audio(&audio_frame(&preset, 0)).unwrap();
    }
    assert!(path.exists());
    cancel.cancel();
    assert!(matches!(
        ex.push_video_nv12(&frame),
        Err(ExportError::Cancelled)
    ));
    drop(ex);
    assert!(!path.exists(), "cancelled export left a file");

    let path = dir.join("errored.mp4");
    let mut ex = Exporter::new(&path, &preset, ExportOptions::default()).unwrap();
    assert!(matches!(
        ex.push_video_nv12(&[0; 10]),
        Err(ExportError::InvalidInput(_))
    ));
    drop(ex);
    assert!(!path.exists(), "failed export left a file");
}

struct Vmaf {
    mean: f64,
    p1: f64,
    ssim: f64,
}

/// VMAF (vmaf_v0.6.1) + float_ssim of `path` vs the regenerated lossless pattern.
fn vmaf(path: &Path, preset: &Preset, frames: u64) -> Option<Vmaf> {
    let filters = Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .ok()?;
    if !String::from_utf8_lossy(&filters.stdout).contains("libvmaf") {
        eprintln!("SKIP VMAF: this ffmpeg has no libvmaf");
        return None;
    }
    let dir = path.parent().unwrap();
    let log = format!("{}.vmaf.json", path.file_stem().unwrap().to_string_lossy());
    let (w, h) = (preset.width as usize, preset.height as usize);
    let mut child = Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-hide_banner", "-v", "error", "-y", "-i"])
        .arg(path.file_name().unwrap())
        .args(["-f", "rawvideo", "-pix_fmt", "nv12", "-s", &format!("{w}x{h}")])
        .args(["-r", &format!("{}/{}", preset.fps_num, preset.fps_den), "-i", "-"])
        .arg("-lavfi")
        .arg(format!(
            // Tag the raw reference like the export (else ffmpeg range-converts it) and
            // pair frames by index (the inputs have different timebases).
            "[0:v]format=yuv420p,settb=1,setpts=N[d];\
             [1:v]setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709,\
             format=yuv420p,settb=1,setpts=N[r];\
             [d][r]libvmaf=model=version=vmaf_v0.6.1:feature=name=float_ssim:log_fmt=json:log_path={log}:n_threads=8"
        ))
        .args(["-f", "null", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        for n in 0..frames {
            if stdin.write_all(&pattern(w, h, n)).is_err() {
                break;
            }
        }
    });
    let status = child.wait().unwrap();
    writer.join().unwrap();
    assert!(status.success(), "ffmpeg libvmaf failed");
    let json: Value = serde_json::from_slice(&std::fs::read(dir.join(&log)).unwrap()).unwrap();
    let _ = std::fs::remove_file(dir.join(&log));
    let mut per_frame: Vec<f64> = json["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["metrics"]["vmaf"].as_f64().unwrap())
        .collect();
    assert_eq!(per_frame.len() as u64, frames, "VMAF frame count");
    per_frame.sort_by(f64::total_cmp);
    Some(Vmaf {
        mean: json["pooled_metrics"]["vmaf"]["mean"].as_f64().unwrap(),
        p1: per_frame[per_frame.len() / 100],
        ssim: json["pooled_metrics"]["float_ssim"]["mean"]
            .as_f64()
            .unwrap(),
    })
}

/// A-QUAL-01/02 + A-PERF-03/04 on a 5 s synthetic pattern (milestone only, slow).
#[test]
#[ignore]
fn quality_and_speed_1080p_4k() {
    let _mf = mf_lock();
    if !tool("ffmpeg") || !tool("ffprobe") {
        eprintln!("SKIP: ffmpeg/ffprobe not on PATH");
        return;
    }
    let mut failures = Vec::new();
    for id in ["master-1080p", "master-4k"] {
        let preset = Preset::builtin_by_id(id).unwrap();
        let frames = 150;
        let secs = preset.frame_time_hns(frames) as f64 / 1e7;
        let dir = out_dir("quality");
        for (i, enc) in h264_encoders().iter().enumerate() {
            let path = dir.join(format!("{id}-enc{i}.mp4"));
            let (spent, used) = encode(&path, &preset, enc, frames);
            let rt = secs / spent.as_secs_f64();
            let mut bad = conformance(&path, &preset, frames);
            let q = vmaf(&path, &preset, frames);
            if let Some(q) = &q {
                if q.mean < 93.0 || q.p1 < 85.0 || q.ssim < 0.97 {
                    bad.push("quality below A-QUAL-01/02".into());
                }
                println!(
                    "{id} {used}: {rt:.2}x realtime, VMAF mean {:.2} p1 {:.2}, SSIM {:.4}, {} KiB",
                    q.mean,
                    q.p1,
                    q.ssim,
                    std::fs::metadata(&path).unwrap().len() / 1024
                );
            }
            // A-PERF-03 (1080p >= 2x) / A-PERF-04 (4K on QSV >= 0.75x): hardware only.
            let min_rt = if preset.width > 1920 { 0.75 } else { 2.0 };
            if enc.hardware && rt < min_rt {
                bad.push(format!("{rt:.2}x realtime < {min_rt}x"));
            }
            if !bad.is_empty() {
                failures.push(format!("{id} {}: {bad:?}", enc.name));
            }
            let _ = std::fs::remove_file(&path);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
