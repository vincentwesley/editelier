//! GPU tests: neutral grade round trip (A-COL-01) and the title golden frame (A-QUAL-03).
//! Skip with a message when no DX12 adapter is present. `UPDATE_GOLDEN=1` rewrites goldens.

use editelier_core::{Tick, TitleClip, TitleStyle};
use editelier_render::*;
use std::path::{Path, PathBuf};

fn gpu() -> Option<Gpu> {
    Gpu::headless().map_err(|e| eprintln!("SKIP: {e}")).ok()
}

/// Smooth in-gamut test image: hue/value ramps that stay away from clipping.
fn ramp(w: usize, h: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let r = 40.0 + 170.0 * fx;
            let g = 30.0 + 180.0 * fy;
            let b = 200.0 - 150.0 * (fx * 0.6 + fy * 0.4);
            v.extend([r as u8, g as u8, b as u8, 255]);
        }
    }
    v
}

#[test]
fn neutral_grade_round_trips_nv12_709_limited() {
    let Some(gpu) = gpu() else { return };
    let (w, h) = (320usize, 180usize);
    let nv12 = editelier_export::rgba_to_nv12(&ramp(w, h), w, h).unwrap();
    let mut r = Renderer::new(&gpu, Fonts::bundled());
    r.upload(&SourceFrame {
        width: w as u32,
        height: h as u32,
        pixels: Pixels::Nv12 {
            data: &nv12,
            stride: w,
        },
        color: SourceColor {
            matrix: Matrix::Bt709,
            full_range: false,
        },
    })
    .unwrap();
    r.compose((w as u32, h as u32), &[], 1.0);
    let out = r.read_nv12().unwrap();
    let worst = nv12
        .iter()
        .zip(&out)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 1, "max code difference {worst}");
}

#[test]
fn neutral_grade_round_trips_rgba_still() {
    let Some(gpu) = gpu() else { return };
    let (w, h) = (256usize, 64usize);
    // Every code value 0..=255 per channel, plus a smooth ramp: still images use the full range.
    let mut img = ramp(w, h);
    for x in 0..w {
        img[x * 4..x * 4 + 3].copy_from_slice(&[x as u8, (255 - x) as u8, (x * 7 % 256) as u8]);
    }
    let mut r = Renderer::new(&gpu, Fonts::bundled());
    r.upload(&SourceFrame {
        width: w as u32,
        height: h as u32,
        pixels: Pixels::Rgba8 {
            data: &img,
            stride: w * 4,
        },
        color: SourceColor {
            matrix: Matrix::Bt709,
            full_range: true,
        },
    })
    .unwrap();
    r.compose((w as u32, h as u32), &[], 1.0);
    let out = r.read_rgba8().unwrap();
    let worst = img
        .iter()
        .zip(&out)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 1, "max difference {worst}/255");
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn read_png(p: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).ok()?));
    let mut rd = dec.read_info().ok()?;
    let mut buf = vec![0; rd.output_buffer_size()?];
    let info = rd.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

fn write_png(p: &Path, w: u32, h: u32, rgba: &[u8]) {
    let f = std::io::BufWriter::new(std::fs::File::create(p).unwrap());
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// A-QUAL-03 compare: max Δ ≤ 2/255 on ≥ 99.9 % of pixels and luma SSIM ≥ 0.995. On failure
/// writes the actual frame and a ×8 diff next to the test binary.
fn check_golden(name: &str, w: u32, h: u32, actual: &[u8]) {
    let path = golden_dir().join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() || !path.is_file() {
        write_png(&path, w, h, actual);
        eprintln!("wrote golden {}", path.display());
        return;
    }
    let (gw, gh, golden) = read_png(&path).expect("golden png");
    assert_eq!((gw, gh), (w, h), "golden size");
    let px = (w * h) as usize;
    let bad = (0..px)
        .filter(|i| (0..3).any(|c| golden[i * 4 + c].abs_diff(actual[i * 4 + c]) > 2))
        .count();
    let luma = |v: &[u8]| -> Vec<u8> {
        v.chunks(4)
            .map(|p| ((p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8) as u8)
            .collect()
    };
    let s = ssim(&luma(&golden), &luma(actual), w as usize, h as usize);
    let ok = bad * 1000 <= px && s >= 0.995;
    if !ok {
        let out = std::env::temp_dir().join("editelier-render-golden");
        std::fs::create_dir_all(&out).unwrap();
        write_png(&out.join(format!("{name}.actual.png")), w, h, actual);
        let diff: Vec<u8> = golden
            .iter()
            .zip(actual)
            .enumerate()
            .map(|(i, (a, b))| {
                if i % 4 == 3 {
                    255
                } else {
                    a.abs_diff(*b).saturating_mul(8)
                }
            })
            .collect();
        write_png(&out.join(format!("{name}.diff.png")), w, h, &diff);
        panic!(
            "{name}: {bad} pixels off by > 2, SSIM {s:.4}; see {}",
            out.display()
        );
    }
}

#[test]
fn title_golden_frame() {
    let Some(gpu) = gpu() else { return };
    let mut r = Renderer::new(&gpu, Fonts::bundled());
    let title = TitleClip {
        id: 1,
        start: Tick::ZERO,
        duration: Tick(1),
        text: "Editelier 0123\nTitles in Space Grotesk".into(),
        style: TitleStyle {
            font: "Space Grotesk".into(),
            size: 40.0,
            color: [255, 214, 120, 255],
        },
        position: [0.5, 0.5],
    };
    let lower = TitleClip {
        id: 2,
        text: "Inter lower third".into(),
        style: TitleStyle {
            font: "Inter".into(),
            size: 24.0,
            color: [255, 255, 255, 200],
        },
        position: [0.5, 0.85],
        ..title.clone()
    };
    r.compose((640, 360), &[&title, &lower], 1.0);
    let out = r.read_rgba8().unwrap();
    // Something was drawn, in the title colour.
    assert!(out.chunks(4).any(|p| p == [255, 214, 120, 255]));
    check_golden("title", 640, 360, &out);
}

#[test]
fn letterbox_keeps_bars_black() {
    let Some(gpu) = gpu() else { return };
    let img = [200u8, 200, 200, 255].repeat(160 * 90);
    let mut r = Renderer::new(&gpu, Fonts::bundled());
    r.upload(&SourceFrame {
        width: 160,
        height: 90,
        pixels: Pixels::Rgba8 {
            data: &img,
            stride: 640,
        },
        color: SourceColor {
            matrix: Matrix::Bt709,
            full_range: true,
        },
    })
    .unwrap();
    r.compose((90, 160), &[], 1.0);
    let out = r.read_rgba8().unwrap();
    let px = |x: usize, y: usize| &out[(y * 90 + x) * 4..(y * 90 + x) * 4 + 3];
    assert_eq!(px(45, 5), [0, 0, 0]);
    assert_eq!(px(45, 80), [200, 200, 200]);
}
