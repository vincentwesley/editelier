//! A-CUT-01..04 lossless path: the editor model (editelier-core) makes the cut, Media Foundation
//! decodes every timeline frame (DXVA + readback, as the app), the shipped LGPL FFmpeg helper
//! writes FFV1/MKV, and its framemd5 (oracle ffmpeg) must equal the oracle's own `trim` of the
//! sources, frame for frame. Skips without media (scripts/make-test-media.ps1), helper or oracle.
#![cfg(windows)]

use editelier_core::{Command as Cmd, FrameRate, Id, MediaMeta, Project, Session, Settings, Tick};
use editelier_media::win::{self, VideoDecoder};
use editelier_media::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FPS: FrameRate = FrameRate::FPS_30;

fn oracle() -> String {
    std::env::var("EDITELIER_ORACLE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

struct Src {
    path: PathBuf,
    ix: Index,
    dec: VideoDecoder,
    size: (u32, u32),
}

fn open(t: &Tools, name: &str) -> Option<Src> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-media")
        .join(name);
    if !path.is_file() {
        eprintln!("SKIP: {} missing", path.display());
        return None;
    }
    let v = probe(t, &path).unwrap().video.remove(0);
    let cache = std::env::temp_dir().join("editelier-cut-tests").join(name);
    let ix = ffmpeg::index(t, &path, &v, &cache).unwrap();
    let device = win::create_device().unwrap();
    let mut dec = VideoDecoder::open(&path, &v, ix.clone(), Some(&device)).unwrap();
    dec.set_cpu_readback(true);
    Some(Src {
        path,
        ix,
        dec,
        size: (v.width, v.height),
    })
}

/// Same tick -> PTS mapping as the shell (`library::pts_at`): sample-and-hold from the first PTS.
fn pts_at(ix: &Index, t: Tick) -> i64 {
    let tb = ix.time_base;
    let den = tb.num as i128 * editelier_core::TICKS_PER_SECOND as i128;
    let off = (t.0 as i128 * tb.den as i128 + den / 2).div_euclid(den) as i64;
    ix.pts[ix.frame_at(ix.pts[0] + off)]
}

/// Tightly packed NV12 (Y rows then UV rows, `width` bytes each).
fn nv12(f: &Frame) -> Vec<u8> {
    let FrameData::Cpu { bytes, stride } = &f.data else {
        panic!("expected a CPU frame");
    };
    assert_eq!(f.format, PixelFormat::Nv12);
    let (w, h) = (f.width as usize, f.height as usize);
    bytes
        .chunks(*stride)
        .take(h + h / 2)
        .flat_map(|r| &r[..w])
        .copied()
        .collect()
}

fn hashes(framemd5: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(framemd5)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_owned()))
        .collect()
}

fn framemd5(input: &Path, trim: Option<(i64, i64)>) -> Vec<String> {
    let mut c = Command::new(oracle());
    c.args(["-v", "error", "-noautorotate", "-i"])
        .arg(input)
        .args(["-map", "0:v:0"]);
    if let Some((a, b)) = trim {
        c.args(["-vf", &format!("trim=start_frame={a}:end_frame={b}")]);
    }
    let o = c
        .args([
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "nv12",
            "-f",
            "framemd5",
            "-",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    hashes(&o.stdout)
}

/// Session at 30 fps with `srcs` added as media 1, 2, ...
fn session(srcs: &[&Src]) -> Session {
    let mut s = Session::new(Project::new(Settings {
        fps: FPS,
        ..Settings::default()
    }));
    for src in srcs {
        let meta = MediaMeta {
            kind: editelier_core::MediaKind::Video,
            duration: FPS.tick_of(src.ix.pts.len() as i64),
            fps: Some(FPS),
            is_vfr: false,
            has_audio: false,
        };
        s.execute(&Cmd::AddMedia {
            path: src.path.display().to_string(),
            meta,
        })
        .unwrap();
    }
    s
}

fn insert(s: &mut Session, media: Id, a: i64, b: i64) -> Id {
    s.execute(&Cmd::InsertClip {
        media,
        source_in: FPS.tick_of(a),
        source_out: FPS.tick_of(b),
        index: None,
    })
    .unwrap()
    .unwrap()
}

/// Exports the main track losslessly (MF decode -> FFV1/MKV via the helper) and compares its
/// framemd5 with the oracle's trim of the sources; also checks the model's source ranges.
fn check(name: &str, tools: &Tools, s: &Session, srcs: &mut [Src], want: &[(usize, i64, i64)]) {
    let p = s.project();
    let ranges: Vec<(usize, i64, i64)> = p
        .timeline
        .main
        .iter()
        .map(|c| {
            (
                c.media as usize - 1,
                FPS.frame_at(c.source_in),
                FPS.frame_at(c.source_out),
            )
        })
        .collect();
    assert_eq!(ranges, want, "{name}: model source ranges");
    let total: i64 = want.iter().map(|r| r.2 - r.1).sum();
    assert_eq!(FPS.frame_at(p.duration()), total, "{name}: timeline length");

    let (w, h) = srcs[0].size;
    let out = std::env::temp_dir()
        .join("editelier-cut-tests")
        .join(format!("{name}.mkv"));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut enc = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "nv12"])
        .args([
            "-s",
            &format!("{w}x{h}"),
            "-framerate",
            "30",
            "-i",
            "pipe:0",
        ])
        .args(["-c:v", "ffv1", "-level", "3"])
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = enc.stdin.take().unwrap();
    for c in p.timeline.main.iter() {
        let src = &mut srcs[c.media as usize - 1];
        assert_eq!(src.size, (w, h), "{name}: joined clips must share a format");
        for k in 0..FPS.frame_at(c.duration()) {
            let pts = pts_at(&src.ix, c.source_in + FPS.tick_of(k));
            let f = src.dec.frame_at(pts).unwrap();
            assert_eq!(f.pts, pts);
            stdin.write_all(&nv12(&f)).unwrap();
        }
    }
    drop(stdin);
    assert!(enc.wait().unwrap().success(), "{name}: FFV1 encode");

    let got = framemd5(&out, None);
    let mut expect = Vec::new();
    for &(i, a, b) in want {
        expect.extend(framemd5(&srcs[i].path, Some((a, b))));
    }
    assert_eq!(got.len(), total as usize, "{name}: exported frame count");
    assert_eq!(
        expect.len(),
        total as usize,
        "{name}: reference frame count"
    );
    let first_bad = got.iter().zip(&expect).position(|(a, b)| a != b);
    assert_eq!(
        first_bad, None,
        "{name}: framemd5 differs from the oracle cut at timeline frame {first_bad:?}"
    );
    eprintln!("{name}: {total} frames framemd5-identical");
    let _ = std::fs::remove_file(&out);
}

fn ready() -> Option<Tools> {
    let t = Tools::locate().map_err(|e| eprintln!("SKIP: {e}")).ok()?;
    let oracle_ok = Command::new(oracle())
        .arg("-version")
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    oracle_ok.then_some(t).or_else(|| {
        eprintln!("SKIP: oracle ffmpeg not on PATH");
        None
    })
}

/// A-CUT-01: 20 seeded random trim pairs on a GOP-250 clip, all on one timeline.
#[test]
fn a_cut_01_trims_long_gop() {
    let Some(t) = ready() else { return };
    let Some(src) = open(&t, "cfr30_longgop_720p_h264.mp4") else {
        return;
    };
    let n = src.ix.pts.len() as i64;
    let mut s = session(&[&src]);
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut next = |m: i64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % m as u64) as i64
    };
    let mut want = Vec::new();
    for _ in 0..20 {
        let a = next(n - 1);
        let b = (a + 1 + next(30)).min(n);
        // Insert the whole clip, then trim both edges like a drag would.
        let id = insert(&mut s, 1, 0, n);
        s.execute(&Cmd::TrimStart {
            id,
            source_in: FPS.tick_of(a),
            drag: None,
        })
        .unwrap();
        s.execute(&Cmd::TrimEnd {
            id,
            source_out: FPS.tick_of(b),
            drag: None,
        })
        .unwrap();
        want.push((0, a, b));
    }
    check("a-cut-01", &t, &s, &mut [src], &want);
}

/// A-CUT-02: split at frame 37 on B-frames + edit list (+ AAC priming); the seam is exact.
#[test]
fn a_cut_02_split_bframes_editlist() {
    let Some(t) = ready() else { return };
    let Some(src) = open(&t, "av_offset_h264_aac.mp4") else {
        return;
    };
    let n = src.ix.pts.len() as i64;
    let mut s = session(&[&src]);
    insert(&mut s, 1, 0, n);
    s.execute(&Cmd::Split {
        at: FPS.tick_of(37),
    })
    .unwrap();
    check("a-cut-02", &t, &s, &mut [src], &[(0, 0, 37), (0, 37, n)]);
}

/// A-CUT-03: two clips from different files (same format) back to back.
#[test]
fn a_cut_03_join_two_files() {
    let Some(t) = ready() else { return };
    let (Some(a), Some(b)) = (
        open(&t, "cfr30_1080p_h264_aac.mp4"),
        open(&t, "bench20_1080p30.mp4"),
    ) else {
        return;
    };
    let mut s = session(&[&a, &b]);
    insert(&mut s, 1, 70, 120);
    insert(&mut s, 2, 0, 40);
    check("a-cut-03", &t, &s, &mut [a, b], &[(0, 70, 120), (1, 0, 40)]);
}

/// A-CUT-04: ripple delete the middle third; the tail shifts by exactly the deleted length.
#[test]
fn a_cut_04_ripple_delete() {
    let Some(t) = ready() else { return };
    let Some(src) = open(&t, "cfr30_1080p_h264_aac.mp4") else {
        return;
    };
    let mut s = session(&[&src]);
    insert(&mut s, 1, 0, 120);
    for f in [30, 60] {
        s.execute(&Cmd::Split { at: FPS.tick_of(f) }).unwrap();
    }
    let mid = s.project().timeline.main[1].id;
    s.execute(&Cmd::RippleDelete { id: mid }).unwrap();
    check("a-cut-04", &t, &s, &mut [src], &[(0, 0, 30), (0, 60, 120)]);
}
