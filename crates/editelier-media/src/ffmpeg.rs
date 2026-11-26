//! FFmpeg (LGPL shared build) as a helper process: probe, PTS/keyframe index, and decode of
//! royalty-free codecs only.
//!
//! ponytail: helper process (ffprobe/ffmpeg.exe) instead of in-process rsmpeg: no libclang/bindgen
//! in the build and no unsafe FFI; costs ~30-80 ms process start per call. Move to rsmpeg when we
//! need in-process demux -> MF handoff or per-frame calls.

use crate::{
    AudioStream, ColorInfo, Error, Frame, FrameData, Hdr, ImageInfo, Index, MICROS, MediaKind,
    PixelFormat, ProbeInfo, Rational, Result, VideoStream, detect_vfr,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Codecs FFmpeg may decode for us. H.264, HEVC and AAC are never on this list (D3): they go
/// through Media Foundation only.
pub const ROYALTY_FREE: &[&str] = &[
    "vp8", "vp9", "av1", "opus", "vorbis", "flac", "mp3", "png", "mjpeg", "theora",
];

fn royalty_free(codec: &str) -> bool {
    ROYALTY_FREE.contains(&codec) || codec.starts_with("pcm_")
}

/// Paths to the LGPL `ffprobe.exe` / `ffmpeg.exe`.
#[derive(Clone, Debug)]
pub struct Tools {
    pub ffprobe: PathBuf,
    pub ffmpeg: PathBuf,
}

impl Tools {
    pub fn in_dir(dir: &Path) -> Self {
        let exe = |n: &str| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX));
        Self {
            ffprobe: exe("ffprobe"),
            ffmpeg: exe("ffmpeg"),
        }
    }

    /// `EDITELIER_FFMPEG_DIR`, then beside the running exe, then (debug builds) the repo's
    /// `third_party/ffmpeg/bin` from `scripts/fetch-ffmpeg.ps1`.
    pub fn locate() -> Result<Self> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("EDITELIER_FFMPEG_DIR") {
            dirs.push(d.into());
        }
        if let Some(d) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            dirs.push(d);
        }
        #[cfg(debug_assertions)]
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ffmpeg/bin"));
        dirs.into_iter()
            .map(|d| Self::in_dir(&d))
            .find(|t| t.ffprobe.is_file() && t.ffmpeg.is_file())
            .ok_or_else(|| {
                Error::Tool("FFmpeg helper not found; run scripts/fetch-ffmpeg.ps1 or set EDITELIER_FFMPEG_DIR".into())
            })
    }
}

/// Input option for every helper run: local files only, so a crafted HLS/concat/playlist file
/// can't make FFmpeg fetch anything over the network.
const LOCAL_ONLY: [&str; 2] = ["-protocol_whitelist", "file"];

/// Runs a tool without a console window and returns stdout.
fn run(exe: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let mut cmd = Command::new(exe);
    cmd.args(LOCAL_ONLY).args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| Error::Tool(format!("could not run {}: {e}", exe.display())))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Unsupported(format!(
            "this file couldn't be read: {}",
            msg.lines().next().unwrap_or("unknown error")
        )));
    }
    Ok(out.stdout)
}

macro_rules! os_args {
    ($($a:expr),* $(,)?) => { &[$(std::ffi::OsStr::new($a)),*] };
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k)?
        .as_str()
        .filter(|s| !s.is_empty() && *s != "unknown")
        .map(str::to_owned)
}

fn num_of(v: &Value, k: &str) -> Option<f64> {
    let x = v.get(k)?;
    x.as_f64().or_else(|| x.as_str()?.parse().ok())
}

fn micros(v: &Value) -> Option<i64> {
    num_of(v, "duration").map(|s| (s * 1e6).round() as i64)
}

fn rotation(stream: &Value) -> u16 {
    let side = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find_map(|s| num_of(s, "rotation")));
    // The legacy `rotate` tag is clockwise; the display matrix rotation is counter-clockwise.
    let tag = stream
        .get("tags")
        .and_then(|t| num_of(t, "rotate"))
        .map(|d| -d);
    let deg = side.or(tag).unwrap_or(0.0).round() as i64;
    (((deg % 360) + 360) % 360) as u16
}

/// Probes container, streams, fps, VFR, rotation, colour and audio parameters.
pub fn probe(tools: &Tools, path: &Path) -> Result<ProbeInfo> {
    let json = run(
        &tools.ffprobe,
        &[
            "-v".as_ref(),
            "error".as_ref(),
            "-show_format".as_ref(),
            "-show_streams".as_ref(),
            "-of".as_ref(),
            "json".as_ref(),
            path.as_os_str(),
        ],
    )?;
    let root: Value = serde_json::from_slice(&json)
        .map_err(|e| Error::Tool(format!("ffprobe output unreadable: {e}")))?;
    let format = &root["format"];
    let container = str_of(format, "format_name").unwrap_or_default();
    let mut video = Vec::new();
    let mut audio = Vec::new();
    for s in root["streams"].as_array().into_iter().flatten() {
        let index = s["index"].as_u64().unwrap_or(0) as u32;
        let codec = str_of(s, "codec_name").unwrap_or_default();
        match s["codec_type"].as_str() {
            Some("video") if s["disposition"]["attached_pic"].as_u64() != Some(1) => {
                let transfer = str_of(s, "color_transfer");
                let hdr = match transfer.as_deref() {
                    Some("arib-std-b67") => Some(Hdr::Hlg),
                    Some("smpte2084") => Some(Hdr::Pq),
                    _ => None,
                };
                video.push(VideoStream {
                    index,
                    codec,
                    pix_fmt: str_of(s, "pix_fmt"),
                    width: s["width"].as_u64().unwrap_or(0) as u32,
                    height: s["height"].as_u64().unwrap_or(0) as u32,
                    time_base: str_of(s, "time_base")
                        .and_then(|t| Rational::parse(&t))
                        .unwrap_or(MICROS),
                    avg_fps: str_of(s, "avg_frame_rate").and_then(|t| Rational::parse(&t)),
                    r_fps: str_of(s, "r_frame_rate").and_then(|t| Rational::parse(&t)),
                    vfr: false,
                    rotation: rotation(s),
                    color: ColorInfo {
                        primaries: str_of(s, "color_primaries"),
                        transfer,
                        matrix: str_of(s, "color_space"),
                        range: str_of(s, "color_range"),
                        hdr,
                    },
                    duration_us: micros(s),
                });
            }
            Some("audio") => audio.push(AudioStream {
                index,
                codec,
                sample_rate: num_of(s, "sample_rate").unwrap_or(0.0) as u32,
                channels: s["channels"].as_u64().unwrap_or(0) as u32,
                channel_layout: str_of(s, "channel_layout"),
                duration_us: micros(s),
                start_us: num_of(s, "start_time").map(|t| (t * 1e6).round() as i64),
            }),
            _ => {}
        }
    }
    // A single-frame "video" from an image demuxer is a still.
    if (container.contains("image2") || container.ends_with("_pipe"))
        && let Some(v) = video.first()
    {
        let image = ImageInfo {
            width: v.width,
            height: v.height,
            orientation: 1,
            format: v.codec.clone(),
        };
        return Ok(ProbeInfo {
            path: path.into(),
            kind: MediaKind::Image,
            container,
            duration_us: None,
            video: Vec::new(),
            audio: Vec::new(),
            image: Some(image),
        });
    }
    if let Some(v) = video.first_mut() {
        // VFR from the first 600 packets of the main video stream.
        let first = packets(tools, path, v.index, Some(600))?;
        let mut pts: Vec<i64> = first.iter().map(|p| p.0).collect();
        pts.sort_unstable();
        v.vfr = detect_vfr(&pts);
    }
    let kind = match (video.is_empty(), audio.is_empty()) {
        (false, _) => MediaKind::Video,
        (true, false) => MediaKind::Audio,
        _ => {
            return Err(Error::Unsupported(
                "no audio or video found in this file".into(),
            ));
        }
    };
    Ok(ProbeInfo {
        path: path.into(),
        kind,
        container,
        duration_us: micros(format),
        video,
        audio,
        image: None,
    })
}

/// (pts, keyframe) per packet of `stream`, discarded packets (edit lists) dropped.
fn packets(
    tools: &Tools,
    path: &Path,
    stream: u32,
    limit: Option<u32>,
) -> Result<Vec<(i64, bool)>> {
    let sel = format!("{stream}");
    let interval = limit.map(|n| format!("%+#{n}"));
    let mut args: Vec<&std::ffi::OsStr> = os_args!["-v", "error", "-select_streams", &sel].to_vec();
    if let Some(i) = &interval {
        args.extend_from_slice(os_args!["-read_intervals", i]);
    }
    args.extend_from_slice(os_args![
        "-show_entries",
        "packet=pts,dts,flags",
        "-of",
        "csv=p=0"
    ]);
    args.push(path.as_os_str());
    let out = run(&tools.ffprobe, &args)?;
    let text = String::from_utf8_lossy(&out);
    Ok(text
        .lines()
        .filter_map(|l| {
            let mut it = l.split(',');
            let (pts, dts, flags) = (it.next()?, it.next()?, it.next()?);
            let t = pts.parse().or_else(|_| dts.parse()).ok()?;
            (!flags.contains('D')).then(|| (t, flags.starts_with('K')))
        })
        .collect())
}

/// Builds the frame index for `stream` (or loads it from `cache_dir`).
pub fn index(tools: &Tools, path: &Path, v: &VideoStream, cache_dir: &Path) -> Result<Index> {
    let file = cache_dir.join(format!("{}.s{}.index", crate::cache_key(path)?, v.index));
    if let Ok(text) = std::fs::read_to_string(&file)
        && let Some(ix) = parse_index(&text, v.index)
    {
        return Ok(ix);
    }
    let pk = packets(tools, path, v.index, None)?;
    if pk.is_empty() {
        return Err(Error::Unsupported("the video stream has no frames".into()));
    }
    let mut pts: Vec<i64> = pk.iter().map(|p| p.0).collect();
    pts.sort_unstable();
    pts.dedup();
    let mut keyframes: Vec<i64> = pk.iter().filter(|p| p.1).map(|p| p.0).collect();
    keyframes.sort_unstable();
    if keyframes.is_empty() {
        keyframes.push(pts[0]);
    }
    let ix = Index {
        stream: v.index,
        time_base: v.time_base,
        pts,
        keyframes,
    };
    std::fs::create_dir_all(cache_dir)?;
    let mut text = format!("{}\n", ix.time_base);
    for p in &ix.pts {
        let k = ix.keyframes.binary_search(p).is_ok();
        text.push_str(&format!("{p}{}\n", if k { ",k" } else { "" }));
    }
    crate::cache::write_atomic(&file, text.as_bytes())?;
    Ok(ix)
}

fn parse_index(text: &str, stream: u32) -> Option<Index> {
    let mut lines = text.lines();
    let time_base = Rational::parse(lines.next()?)?;
    let (mut pts, mut keyframes) = (Vec::new(), Vec::new());
    for l in lines {
        let (p, k) = l.split_once(',').map_or((l, false), |(p, _)| (p, true));
        let p: i64 = p.parse().ok()?;
        pts.push(p);
        if k {
            keyframes.push(p);
        }
    }
    (!pts.is_empty() && !keyframes.is_empty()).then_some(Index {
        stream,
        time_base,
        pts,
        keyframes,
    })
}

fn check_royalty_free(codec: &str) -> Result<()> {
    if royalty_free(codec) {
        Ok(())
    } else {
        Err(Error::Unsupported(format!(
            "{codec} needs a Windows codec that isn't available here"
        )))
    }
}

/// Decodes the frame with exactly `pts` as RGBA (royalty-free codecs only, ~100 ms per call).
pub fn frame_at(
    tools: &Tools,
    path: &Path,
    v: &VideoStream,
    index: &Index,
    pts: i64,
) -> Result<Frame> {
    check_royalty_free(&v.codec)?;
    let pts = index.pts[index.frame_at(pts)];
    let first = index.pts[0];
    // Seek a hair before the frame; ffmpeg's accurate seek drops everything earlier.
    let secs = index.time_base.as_f64() * (pts - first) as f64 - 0.0005;
    let ss = format!("{:.6}", secs.max(0.0));
    let map = format!("0:{}", v.index);
    let out = run(
        &tools.ffmpeg,
        &[
            &os_args!["-v", "error", "-noautorotate", "-ss", &ss, "-i"][..],
            &[path.as_os_str()],
            os_args![
                "-map",
                &map,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-"
            ],
        ]
        .concat(),
    )?;
    let need = v.width as usize * v.height as usize * 4;
    if out.len() < need {
        return Err(Error::Codec("frame decode returned no data".into()));
    }
    Ok(Frame {
        pts,
        width: v.width,
        height: v.height,
        format: PixelFormat::Rgba8,
        data: FrameData::Cpu {
            bytes: out[..need].to_vec(),
            stride: v.width as usize * 4,
        },
    })
}

/// Streams an audio stream as f32 48 kHz interleaved chunks to `sink(channels, samples)`
/// (royalty-free codecs only). Sample 0 is presentation time 0 of the file: edit lists and
/// priming are honoured, a late start is padded with silence and anything before 0 is dropped.
pub fn stream_audio(
    tools: &Tools,
    path: &Path,
    a: &AudioStream,
    sink: &mut dyn FnMut(u32, &[f32]) -> Result<()>,
) -> Result<()> {
    use std::io::Read;
    check_royalty_free(&a.codec)?;
    let map = format!("0:{}", a.index);
    let mut cmd = Command::new(&tools.ffmpeg);
    // -copyts keeps the file's timeline; aresample pads/trims so output starts at t = 0.
    cmd.args(LOCAL_ONLY)
        .args(["-v", "error", "-copyts", "-i"])
        .arg(path)
        .args(["-map", &map, "-af", "aresample=48000:async=1:first_pts=0"])
        .args(["-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Tool(format!("could not run {}: {e}", tools.ffmpeg.display())))?;
    let mut out = child.stdout.take().expect("piped");
    let ch = a.channels.max(1);
    let mut buf = vec![0u8; 1 << 16];
    let mut fill = 0;
    let res = loop {
        match out.read(&mut buf[fill..]) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                fill += n;
                let whole = fill / (4 * ch as usize) * (4 * ch as usize);
                if let Err(e) = sink(ch, &crate::f32_le(&buf[..whole])) {
                    break Err(e);
                }
                buf.copy_within(whole..fill, 0);
                fill -= whole;
            }
            Err(e) => break Err(e.into()),
        }
    };
    if res.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    res?;
    if !status.success() {
        return Err(Error::Unsupported("this audio couldn't be decoded".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list_excludes_patent_pool_codecs() {
        for c in ["h264", "hevc", "aac", "aac_fixed", "mpeg4", "mpeg2video"] {
            assert!(!royalty_free(c), "{c}");
        }
        assert!(royalty_free("vp9") && royalty_free("pcm_s16le") && royalty_free("opus"));
    }

    #[test]
    fn index_cache_round_trip() {
        let ix = parse_index("1/30\n0,k\n1\n2,k\n", 3).unwrap();
        assert_eq!(ix.pts, vec![0, 1, 2]);
        assert_eq!(ix.keyframes, vec![0, 2]);
        assert_eq!(ix.stream, 3);
    }
}
