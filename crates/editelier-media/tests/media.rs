//! Integration tests against `test-media/` (scripts/make-test-media.ps1). They skip with a message
//! when the media, the LGPL FFmpeg helper or the oracle ffmpeg is missing.
#![cfg(windows)]

use editelier_media::win::{self, VideoDecoder};
use editelier_media::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn media(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-media")
        .join(name);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!(
            "SKIP: {} missing; run scripts/make-test-media.ps1",
            p.display()
        );
        None
    }
}

fn tools() -> Option<Tools> {
    Tools::locate().map_err(|e| eprintln!("SKIP: {e}")).ok()
}

fn cache_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("editelier-media-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Oracle: the local (GPL, test-only) ffmpeg's luma planes (8-bit, limited range as coded) for
/// frames `ns` in presentation order.
fn oracle_luma(path: &Path, w: u32, h: u32, ten_bit: bool, ns: &[usize]) -> Option<Vec<Vec<u8>>> {
    let ffmpeg = std::env::var("EDITELIER_ORACLE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let sel: Vec<String> = ns.iter().map(|n| format!("eq(n\\,{n})")).collect();
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-noautorotate", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            &format!("select='{}'", sel.join("+")),
        ])
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt"])
        .arg(if ten_bit { "yuv420p10le" } else { "yuv420p" })
        .arg("-")
        .output()
        .map_err(|e| eprintln!("SKIP: oracle ffmpeg not runnable: {e}"))
        .ok()?;
    let bps = if ten_bit { 2 } else { 1 };
    let size = (w * h) as usize * bps * 3 / 2;
    let mut sorted = ns.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(out.stdout.len(), size * sorted.len(), "oracle frame count");
    let luma = |f: &[u8]| -> Vec<u8> {
        let y = &f[..(w * h) as usize * bps];
        if ten_bit {
            y.chunks(2)
                .map(|b| (u16::from_le_bytes([b[0], b[1]]) >> 2) as u8)
                .collect()
        } else {
            y.to_vec()
        }
    };
    let frames: Vec<Vec<u8>> = out.stdout.chunks(size).map(luma).collect();
    Some(
        ns.iter()
            .map(|n| frames[sorted.binary_search(n).unwrap()].clone())
            .collect(),
    )
}

fn mad(a: &[u8], b: &[u8]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64)
        .sum::<u64>() as f64
        / a.len() as f64
}

#[test]
fn probe_corpus() {
    let Some(t) = tools() else { return };
    let cases: &[(&str, MediaKind)] = &[
        ("cfr30_1080p_h264_aac.mp4", MediaKind::Video),
        ("ntsc2997_720p_h264.mp4", MediaKind::Video),
        ("ntsc5994_720p_h264.mp4", MediaKind::Video),
        ("uhd60_h264.mp4", MediaKind::Video),
        ("vfr_jitter_720p_h264.mp4", MediaKind::Video),
        ("hlg10_720p_hevc.mp4", MediaKind::Video),
        ("rot90_h264.mov", MediaKind::Video),
        ("vp9_opus_360p.mkv", MediaKind::Video),
        ("still_alpha.png", MediaKind::Image),
        ("still.jpg", MediaKind::Image),
        ("tone_48k.wav", MediaKind::Audio),
        ("tone_44k.mp3", MediaKind::Audio),
    ];
    let mut total = 0.0;
    for (name, kind) in cases {
        let Some(p) = media(name) else { return };
        let t0 = Instant::now();
        let info = probe(&t, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        total += ms;
        eprintln!("probe {name}: {ms:.0} ms");
        assert_eq!(info.kind, *kind, "{name}");
        let v = info.video.first();
        let fps = |n, d| Some(Rational::new(n, d));
        match *name {
            "cfr30_1080p_h264_aac.mp4" => {
                let v = v.unwrap();
                assert_eq!(
                    (v.width, v.height, v.avg_fps, v.vfr),
                    (1920, 1080, fps(30, 1), false)
                );
                assert!((info.duration_us.unwrap() - 4_000_000).abs() < 50_000);
                assert_eq!(v.color.matrix.as_deref(), Some("bt709"));
                assert_eq!(info.audio[0].sample_rate, 48_000);
            }
            "ntsc2997_720p_h264.mp4" => assert_eq!(v.unwrap().r_fps, fps(30000, 1001)),
            "ntsc5994_720p_h264.mp4" => assert_eq!(v.unwrap().r_fps, fps(60000, 1001)),
            "uhd60_h264.mp4" => {
                assert_eq!((v.unwrap().width, v.unwrap().avg_fps), (3840, fps(60, 1)))
            }
            "vfr_jitter_720p_h264.mp4" => assert!(v.unwrap().vfr),
            "hlg10_720p_hevc.mp4" => {
                assert_eq!(v.unwrap().color.hdr, Some(Hdr::Hlg));
                assert_eq!(v.unwrap().codec, "hevc");
            }
            "rot90_h264.mov" => assert_eq!(v.unwrap().rotation, 90),
            "vp9_opus_360p.mkv" => {
                assert_eq!(v.unwrap().codec, "vp9");
                assert!(!v.unwrap().vfr, "mkv ms rounding must not read as VFR");
                assert_eq!(info.audio[0].codec, "opus");
            }
            "still_alpha.png" => assert_eq!(
                info.image.as_ref().map(|i| (i.width, i.height)),
                Some((640, 480))
            ),
            "still.jpg" => assert_eq!(
                info.image.as_ref().map(|i| (i.width, i.orientation)),
                Some((800, 1))
            ),
            "tone_48k.wav" => assert_eq!(
                (info.audio[0].sample_rate, info.audio[0].channels),
                (48_000, 2)
            ),
            "tone_44k.mp3" => assert_eq!(info.audio[0].sample_rate, 44_100),
            _ => {}
        }
        if let Some(v) = v {
            assert!(v.vfr || v.avg_fps.is_some(), "{name}");
        }
    }
    eprintln!("probe mean: {:.0} ms/file", total / cases.len() as f64);
}

/// Decodes `frames` (presentation indices) through MF and checks each against the oracle's luma:
/// the decoded frame must match frame N far better than N-1 or N+1.
fn check_seeks(name: &str, frames: &[usize], gpu: bool) {
    let (Some(t), Some(p)) = (tools(), media(name)) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let v = &info.video[0];
    let ix = ffmpeg::index(&t, &p, v, &cache_dir(&format!("seek-{name}"))).unwrap();
    let n = ix.pts.len();
    let device = gpu.then(|| win::create_device().unwrap());
    let mut dec = match VideoDecoder::open(&p, v, ix.clone(), device.as_ref()) {
        Ok(d) => d,
        Err(Error::Unsupported(m)) => return eprintln!("SKIP {name}: {m}"),
        Err(e) => panic!("{name}: {e}"),
    };
    dec.set_cpu_readback(true);
    let mut want: Vec<usize> = frames
        .iter()
        .flat_map(|&f| [f.saturating_sub(1), f, (f + 1).min(n - 1)])
        .collect();
    want.sort_unstable();
    want.dedup();
    let ten = v.pix_fmt.as_deref().is_some_and(|f| f.contains("10"));
    let Some(oracle) = oracle_luma(&p, v.width, v.height, ten, &want) else {
        return;
    };
    let at = |f: usize| &oracle[want.binary_search(&f).unwrap()];
    let mut times = Vec::new();
    for &f in frames {
        let t0 = Instant::now();
        let frame = dec
            .frame_at(ix.pts[f])
            .unwrap_or_else(|e| panic!("{name} frame {f}: {e}"));
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(frame.pts, ix.pts[f]);
        let y = frame.luma().unwrap();
        let d = mad(&y, at(f));
        let dp = mad(&y, at(f.saturating_sub(1)));
        let dn = mad(&y, at((f + 1).min(n - 1)));
        assert!(
            d < 1.5,
            "{name} frame {f}: mean abs diff {d:.2} vs oracle (prev {dp:.2}, next {dn:.2})"
        );
        for o in [f.saturating_sub(1), (f + 1).min(n - 1)] {
            if o != f {
                let dn = mad(&y, at(o));
                assert!(
                    d < dn,
                    "{name} frame {f} looks like frame {o} ({d:.2} vs {dn:.2})"
                );
            }
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "{name} ({}): {} seeks, median {:.1} ms, max {:.1} ms",
        if gpu { "DXVA" } else { "CPU" },
        times.len(),
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

#[test]
fn seek_exact_cfr_gpu() {
    check_seeks(
        "cfr30_1080p_h264_aac.mp4",
        &[0, 1, 59, 60, 61, 119, 45, 10, 90, 30, 31, 2],
        true,
    );
}

#[test]
fn seek_exact_cfr_cpu() {
    check_seeks("cfr30_1080p_h264_aac.mp4", &[0, 61, 7, 118], false);
}

#[test]
fn seek_exact_ntsc() {
    check_seeks("ntsc2997_720p_h264.mp4", &[0, 29, 30, 77, 3, 89], true);
    check_seeks("ntsc5994_720p_h264.mp4", &[0, 59, 60, 150, 17], true);
}

#[test]
fn seek_exact_vfr() {
    check_seeks(
        "vfr_jitter_720p_h264.mp4",
        &[0, 5, 6, 40, 41, 99, 12, 70],
        true,
    );
}

#[test]
fn seek_exact_hevc_hlg() {
    check_seeks("hlg10_720p_hevc.mp4", &[0, 13, 40, 2], true);
}

#[test]
fn sequential_decode_fps_1080p() {
    let (Some(t), Some(p)) = (tools(), media("cfr30_1080p_h264_aac.mp4")) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let v = &info.video[0];
    let ix = ffmpeg::index(&t, &p, v, &cache_dir("fps")).unwrap();
    let device = win::create_device().unwrap();
    let mut dec = VideoDecoder::open(&p, v, ix.clone(), Some(&device)).unwrap();
    let t0 = Instant::now();
    for &pts in &ix.pts {
        let f = dec.frame_at(pts).unwrap();
        assert_eq!(f.pts, pts);
        assert!(matches!(f.data, FrameData::D3D11(_)));
    }
    let fps = ix.pts.len() as f64 / t0.elapsed().as_secs_f64();
    eprintln!(
        "1080p30 H.264 sequential DXVA decode: {fps:.0} fps ({} frames)",
        ix.pts.len()
    );
    assert!(fps > 30.0);
}

#[test]
fn vp9_via_mf_or_ffmpeg() {
    let (Some(t), Some(p)) = (tools(), media("vp9_opus_360p.mkv")) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let v = &info.video[0];
    let ix = ffmpeg::index(&t, &p, v, &cache_dir("vp9")).unwrap();
    let frame = match VideoDecoder::open(&p, v, ix.clone(), None) {
        Ok(mut d) => d.frame_at(ix.pts[20]).unwrap(),
        Err(e) => {
            eprintln!("vp9: MF unavailable ({e}), using FFmpeg");
            ffmpeg::frame_at(&t, &p, v, &ix, ix.pts[20]).unwrap()
        }
    };
    assert_eq!(
        (frame.width, frame.height, frame.pts),
        (640, 360, ix.pts[20])
    );
    assert_eq!(frame.to_rgba().unwrap().len(), 640 * 360 * 4);
    // The FFmpeg path refuses patent-pool codecs.
    let h264 = probe(&t, &media("ntsc2997_720p_h264.mp4").unwrap()).unwrap();
    assert!(ffmpeg::frame_at(&t, &h264.path, &h264.video[0], &ix, 0).is_err());
}

#[test]
fn audio_decode_and_waveform() {
    let Some(t) = tools() else { return };
    for (name, secs) in [
        ("tone_48k.wav", 3.0),
        ("tone_44k.mp3", 3.0),
        ("cfr30_1080p_h264_aac.mp4", 4.0),
        ("vp9_opus_360p.mkv", 2.0),
    ] {
        let Some(p) = media(name) else { return };
        let info = probe(&t, &p).unwrap();
        let a = &info.audio[0];
        let buf = win::decode_audio(&t, &p, a).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(buf.sample_rate, 48_000);
        let got = buf.samples.len() as f64 / buf.channels as f64 / 48_000.0;
        assert!((got - secs).abs() < 0.1, "{name}: {got:.3} s");
        let dir = cache_dir(&format!("wave-{name}"));
        let w = cache::waveform(&p, &dir, || Ok(buf.clone())).unwrap();
        assert!(
            (w.peaks.len() as f64 - secs * 100.0).abs() < 10.0,
            "{name}: {} peaks",
            w.peaks.len()
        );
        let max = w.peaks.iter().map(|p| p.1).fold(0f32, f32::max);
        assert!((0.08..0.2).contains(&max), "{name}: peak {max}"); // ffmpeg sine = 1/8 amplitude
        let again = cache::waveform(&p, &dir, || panic!("cache miss")).unwrap();
        assert_eq!(again, w);
    }
}

#[test]
fn thumbnails_cached() {
    let (Some(t), Some(p)) = (tools(), media("cfr30_1080p_h264_aac.mp4")) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let v = &info.video[0];
    let dir = cache_dir("thumbs");
    let ix = ffmpeg::index(&t, &p, v, &dir).unwrap();
    let mut dec = VideoDecoder::open(&p, v, ix.clone(), None).unwrap();
    let t0 = Instant::now();
    let th = cache::thumbnails(&p, &dir, info.duration_us.unwrap(), 1_000_000, 160, |us| {
        dec.frame_at(ix.pts_at_micros(us))
    })
    .unwrap();
    eprintln!(
        "4 thumbnails: {:.0} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(th.len(), 4);
    assert_eq!((th[0].width, th[0].height), (160, 90));
    assert_eq!(th[1].pts, ix.pts[30]);
    let again = cache::thumbnails(&p, &dir, info.duration_us.unwrap(), 1_000_000, 160, |_| {
        panic!("cache miss")
    })
    .unwrap();
    assert_eq!(again, th);
}

#[test]
fn proxy_4k60() {
    let (Some(t), Some(p)) = (tools(), media("uhd60_h264.mp4")) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let v = &info.video[0];
    let dir = cache_dir("proxy");
    let device = win::create_device().unwrap();
    let dur = info.duration_us.unwrap();
    let t0 = Instant::now();
    let mut last = 0.0;
    let out = win::make_proxy(&p, v, dur, &dir, Some(&device), &mut |f| {
        last = f;
        true
    })
    .unwrap();
    let secs = t0.elapsed().as_secs_f64();
    eprintln!(
        "4K60 -> 1080p proxy: {:.2}x realtime ({secs:.2} s for {:.1} s)",
        dur as f64 / 1e6 / secs,
        dur as f64 / 1e6
    );
    assert_eq!(last, 1.0);
    let pi = probe(&t, &out).unwrap();
    let pv = &pi.video[0];
    assert_eq!(
        (pv.codec.as_str(), pv.width, pv.height),
        ("h264", 1920, 1080)
    );
    let pix = ffmpeg::index(&t, &out, pv, &dir).unwrap();
    let src = ffmpeg::index(&t, &p, v, &dir).unwrap();
    assert_eq!(pix.pts.len(), src.pts.len(), "proxy frame count");
    assert!(
        !needs_proxy(pv, &pix),
        "proxy keyframes must be <= 1 s apart"
    );
    assert!(needs_proxy(v, &src));
    // Cached: second call returns at once.
    let again = win::make_proxy(&p, v, dur, &dir, None, &mut |_| panic!("re-encoded")).unwrap();
    assert_eq!(again, out);
    // Cancel cleans up.
    let dir2 = cache_dir("proxy-cancel");
    let r = win::make_proxy(&p, v, dur, &dir2, Some(&device), &mut |_| false);
    assert!(matches!(r, Err(Error::Cancelled)));
    assert!(std::fs::read_dir(&dir2).unwrap().next().is_none());
}

#[test]
fn phone_rotation_probe() {
    let (Some(t), Some(p)) = (tools(), media("phone_rot90cw_h264.mp4")) else {
        return;
    };
    let v = probe(&t, &p).unwrap().video.remove(0);
    // Coded landscape, display matrix "rotate 90 degrees clockwise" (ffprobe rotation -90).
    assert_eq!((v.width, v.height, v.rotation), (640, 360, 270));
    assert_eq!((v.orientation(), v.display_size()), (6, (360, 640)));
}

/// Corner marker of an RGBA frame: R(ed), G(reen), B(lue) or X (background), sampled in the
/// middle of each corner block, in order top-left, top-right, bottom-left, bottom-right.
fn corners(f: &Frame) -> String {
    let rgba = f.to_rgba().unwrap();
    let (w, h) = (f.width as usize, f.height as usize);
    [
        (w / 8, h / 8),
        (w * 7 / 8, h / 8),
        (w / 8, h * 7 / 8),
        (w * 7 / 8, h * 7 / 8),
    ]
    .iter()
    .map(|&(x, y)| {
        let p = &rgba[(y * w + x) * 4..];
        match (p[0] > 150, p[1] > 150, p[2] > 150) {
            (true, false, false) => 'R',
            (false, true, false) => 'G',
            (false, false, true) => 'B',
            _ => 'X',
        }
    })
    .collect()
}

/// Upright corners per EXIF code for the stored marker "RGBX" (EXIF / TIFF 6.0 definitions).
const UPRIGHT: [&str; 8] = [
    "RGBX", "GRXB", "XBGR", "BXRG", "RBGX", "BRXG", "XGBR", "GXRB",
];

fn check_still(name: &str, code: u16) {
    let Some(p) = media(name) else { return };
    let info = win::wic::probe_image(&p).unwrap();
    let img = info.image.unwrap();
    let f = win::wic::decode_image(&p).unwrap();
    let want = if code >= 5 { (240, 320) } else { (320, 240) };
    assert_eq!(img.orientation, code, "{name}");
    assert_eq!((img.width, img.height), want, "{name}: probe size");
    assert_eq!((f.width, f.height), want, "{name}: decoded size");
    assert_eq!(corners(&f), UPRIGHT[code as usize - 1], "{name}");
}

#[test]
fn exif_orientation_jpeg_all_eight() {
    for code in 1..=8 {
        check_still(&format!("orient_{code}.jpg"), code);
    }
}

#[test]
fn exif_orientation_png() {
    check_still("orient_6.png", 6);
}

#[test]
fn audio_start_offset_and_priming() {
    let (Some(t), Some(p)) = (tools(), media("av_offset_h264_aac.mp4")) else {
        return;
    };
    let info = probe(&t, &p).unwrap();
    let ix = ffmpeg::index(&t, &p, &info.video[0], &cache_dir("avsync")).unwrap();
    assert_eq!(ix.pts[0], 0, "first video frame at t = 0");
    let buf = win::decode_audio(&t, &p, &info.audio[0]).unwrap();
    let ch = buf.channels as usize;
    // The burst starts at 1.000 s (sine from phase 0, 0.25 is reached ~4 samples in).
    let onset = buf
        .samples
        .chunks(ch)
        .position(|f| f[0].abs() > 0.25)
        .unwrap();
    eprintln!("A/V offset clip: burst at sample {onset} (want ~48004)");
    assert!((onset as i64 - 48_004).abs() <= 48, "onset {onset}");
}
