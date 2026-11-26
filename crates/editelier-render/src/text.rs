//! Titles: cosmic-text shapes and rasterises (swash) on the CPU into a coverage mask, which the
//! compositor tints. Masks are cached per (text, style, canvas size), so playback re-uses them.
//! ponytail: CPU raster + upload instead of a glyph atlas (glyphon); titles change rarely and
//! the result is deterministic for golden frames. Move to an atlas if many animated titles land.

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, fontdb,
};
use editelier_core::TitleClip;
use std::collections::HashMap;

/// House title fonts (SIL OFL 1.1, `fonts/OFL.txt`), static instances of the house kit's variable
/// fonts: Space Grotesk Bold, Inter SemiBold, JetBrains Mono Bold.
const BUNDLED: &[(&str, &[u8])] = &[
    (
        "Space Grotesk",
        include_bytes!("../fonts/SpaceGrotesk-Bold.ttf"),
    ),
    ("Inter", include_bytes!("../fonts/Inter-SemiBold.ttf")),
    (
        "JetBrains Mono",
        include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    ),
];

/// The font system with only the bundled fonts (no system fonts: identical output everywhere).
pub struct Fonts {
    system: FontSystem,
    /// Our font name -> the family name fontdb read from the file.
    families: Vec<(&'static str, String)>,
}

impl Fonts {
    pub fn bundled() -> Self {
        let mut db = fontdb::Database::new();
        let mut families = Vec::new();
        for (name, data) in BUNDLED {
            let before = db.len();
            db.load_font_data(data.to_vec());
            if let Some(face) = db.faces().nth(before) {
                let fam = face.families.first().map(|f| f.0.clone());
                families.push((*name, fam.unwrap_or_else(|| (*name).into())));
            }
        }
        Fonts {
            system: FontSystem::new_with_locale_and_db("en-US".into(), db),
            families,
        }
    }

    fn family(&self, name: &str) -> &str {
        self.families
            .iter()
            .find(|(n, _)| *n == name)
            .or(self.families.first())
            .map_or("", |(_, f)| f.as_str())
    }
}

#[derive(Clone, PartialEq)]
struct Key {
    text: String,
    font: String,
    size_bits: u32,
    canvas: (u32, u32),
}

struct Mask {
    texture: wgpu::Texture,
    /// Pixel rect of the mask inside the canvas: x, y (top-left), w, h.
    px: (f32, f32, u32, u32),
}

pub(crate) struct TitleCache {
    fonts: Fonts,
    swash: SwashCache,
    cache: HashMap<u64, (Key, Option<Mask>)>,
}

/// Coverage mask of `text`: width, height, bytes (row-major, 1 byte per pixel).
pub(crate) fn raster(
    fonts: &mut Fonts,
    swash: &mut SwashCache,
    text: &str,
    font: &str,
    size: f32,
    max_width: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    let family = fonts.family(font).to_owned();
    let fs = &mut fonts.system;
    let mut buf = Buffer::new(fs, Metrics::new(size, size * 1.2));
    buf.set_size(Some(max_width), None);
    let attrs = Attrs::new().family(Family::Name(&family));
    buf.set_text(text, &attrs, Shaping::Advanced, Some(Align::Center));
    buf.shape_until_scroll(fs, false);
    let (mut w, mut h) = (0f32, 0f32);
    for run in buf.layout_runs() {
        w = w.max(run.line_w);
        h = h.max(run.line_top + run.line_height);
    }
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (mw, mh) = (max_width.ceil() as u32, h.ceil() as u32 + 2);
    let mut mask = vec![0u8; (mw * mh) as usize];
    buf.draw(fs, swash, Color::rgb(255, 255, 255), |x, y, gw, gh, c| {
        for dy in 0..gh as i32 {
            for dx in 0..gw as i32 {
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && (px as u32) < mw && (py as u32) < mh {
                    let o = (py as u32 * mw + px as u32) as usize;
                    mask[o] = mask[o].max(c.a());
                }
            }
        }
    });
    Some((mw, mh, mask))
}

impl TitleCache {
    pub(crate) fn new(fonts: Fonts) -> Self {
        TitleCache {
            fonts,
            swash: SwashCache::new(),
            cache: HashMap::new(),
        }
    }

    /// The mask texture and NDC rect for `t` on a `canvas`-sized frame, or `None` for empty text.
    pub(crate) fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        t: &TitleClip,
        canvas: (u32, u32),
        scale: f32,
    ) -> Option<(wgpu::TextureView, [f32; 4])> {
        let size = (t.style.size * scale).clamp(1.0, 2048.0);
        let key = Key {
            text: t.text.clone(),
            font: t.style.font.clone(),
            size_bits: size.to_bits(),
            canvas,
        };
        // Cache by title id; a changed title replaces its own entry.
        let fresh = self.cache.get(&t.id).is_none_or(|(k, _)| *k != key);
        if fresh {
            let max_w = canvas.0 as f32 * 0.9;
            let mask = raster(
                &mut self.fonts,
                &mut self.swash,
                &t.text,
                &t.style.font,
                size,
                max_w,
            )
            .map(|(w, h, bytes)| {
                let texture = super::texture(
                    device,
                    "title",
                    (w, h),
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                queue.write_texture(
                    texture.as_image_copy(),
                    &bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w),
                        rows_per_image: None,
                    },
                    texture.size(),
                );
                Mask {
                    texture,
                    px: (0.0, 0.0, w, h),
                }
            });
            self.cache.insert(t.id, (key, mask));
        }
        let mask = self.cache.get(&t.id)?.1.as_ref()?;
        let (_, _, w, h) = mask.px;
        let (cw, ch) = (canvas.0 as f32, canvas.1 as f32);
        // Centre on the anchor, kept inside the canvas; whole pixels so glyphs stay crisp.
        let pos = t.position.map(|v| v.clamp(0.0, 1.0));
        let x = (pos[0] * cw - w as f32 / 2.0)
            .clamp(0.0, (cw - w as f32).max(0.0))
            .round();
        let y = (pos[1] * ch - h as f32 / 2.0)
            .clamp(0.0, (ch - h as f32).max(0.0))
            .round();
        let ndc = |px: f32, len: f32| px / len * 2.0 - 1.0;
        let rect = [
            ndc(x, cw),
            -ndc(y + h as f32, ch),
            ndc(x + w as f32, cw),
            -ndc(y, ch),
        ];
        Some((mask.texture.create_view(&Default::default()), rect))
    }
}
