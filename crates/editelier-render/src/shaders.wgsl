// Editelier render passes (D5, D20). Spaces:
//   source Y'CbCr / R'G'B'  --input transform-->  display-linear Rec.709 (BT.1886, gamma 2.4)
//   --inverse DRT-->  scene-linear  --grade-->  scene-linear  --DRT-->  display-linear
//   --compositor (fit, titles)-->  canvas (display-linear, f16)  --output-->  Rec.709 code values.
// SDR sources keep their code values through a neutral grade (A-COL-01).
// The file is split at `//#name` markers; each module is the common part plus one section.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ---- tone curve (DRT): clean-room invertible rational sigmoid on max(R,G,B), ratio-preserving.
// f(x) = x (1 + x / W^2) / (1 + x) maps scene [0, W] onto display [0, 1]; W = 8 (+3 stops).
const DRT_W2: f32 = 64.0;

fn drt_curve(x: f32) -> f32 {
    return x * (1.0 + x / DRT_W2) / (1.0 + x);
}

// Exact inverse, in the cancellation-free form.
fn drt_curve_inv(y: f32) -> f32 {
    let a = 1.0 - y;
    return 2.0 * y / (a + sqrt(a * a + 4.0 * y / DRT_W2));
}

fn scale_norm(c: vec3<f32>, f: f32) -> vec3<f32> {
    let n = max(c.r, max(c.g, c.b));
    if (n <= 0.0) {
        return c;
    }
    return c * (f / n);
}

fn inverse_drt(display: vec3<f32>) -> vec3<f32> {
    let n = clamp(max(display.r, max(display.g, display.b)), 0.0, 1.0);
    return scale_norm(display, drt_curve_inv(n));
}

fn drt(scene: vec3<f32>) -> vec3<f32> {
    let n = clamp(max(scene.r, max(scene.g, scene.b)), 0.0, 8.0);
    return scale_norm(scene, drt_curve(n));
}

// BT.1886 with Lw = 1, Lb = 0: code value <-> display light (sign-preserving on the way in).
fn to_display_linear(v: vec3<f32>) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(2.4));
}

fn to_code(l: vec3<f32>) -> vec3<f32> {
    return pow(clamp(l, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.4));
}

//#clip
struct ClipU {
    // Y'CbCr -> R'G'B' matrix coefficients.
    kr: f32,
    kb: f32,
    // Sample -> code value on the 8-bit scale (255 for 8-bit, 65535/256 for P010).
    code_scale: f32,
    // 0 = limited range, 1 = full range.
    full_range: f32,
    // 0 = Y'CbCr planes (a = Y, b = CbCr), 1 = R'G'B'A texture (a, straight alpha).
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var tex_a: texture_2d<f32>;
@group(0) @binding(1) var tex_b: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> clip_u: ClipU;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

// Per-clip grade in scene-linear. Fix -> Look -> Film go here, in that fixed order (D20).
// M1 ships the neutral grade only.
fn grade(scene: vec3<f32>) -> vec3<f32> {
    return scene;
}

fn ycbcr_to_rgb(y: f32, cb: f32, cr: f32) -> vec3<f32> {
    let kr = clip_u.kr;
    let kb = clip_u.kb;
    let kg = 1.0 - kr - kb;
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / kg;
    return vec3<f32>(r, g, b);
}

// Clip pass, at source size: source -> input transform -> inverse DRT -> grade -> DRT.
@fragment fn fs_clip(in: VsOut) -> @location(0) vec4<f32> {
    let a = textureSample(tex_a, samp, in.uv);
    let b = textureSample(tex_b, samp, in.uv);
    var code = a.rgb;
    var alpha = a.a;
    if (clip_u.mode == 0u) {
        let s = clip_u.code_scale;
        let yc = a.r * s;
        let cc = b.rg * s;
        alpha = 1.0;
        if (clip_u.full_range > 0.5) {
            code = ycbcr_to_rgb(yc / 255.0, (cc.r - 128.0) / 255.0, (cc.g - 128.0) / 255.0);
        } else {
            code = ycbcr_to_rgb((yc - 16.0) / 219.0, (cc.r - 128.0) / 224.0, (cc.g - 128.0) / 224.0);
        }
    }
    let out = drt(grade(inverse_drt(to_display_linear(code))));
    // Straight alpha in, premultiplied out (the compositor blends premultiplied).
    return vec4<f32>(out * alpha, alpha);
}

//#quad
struct QuadU {
    // NDC rect: x0, y0 (bottom), x1, y1 (top).
    rect: vec4<f32>,
    // Premultiplied display-linear tint (titles).
    tint: vec4<f32>,
    // fs_quad: 0 = premultiplied RGBA texture (clips), 1 = coverage mask in .r (titles).
    // fs_out: 1 = the target is an sRGB format (the hardware re-encodes, so pre-decode).
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var quad_tex: texture_2d<f32>;
@group(0) @binding(1) var quad_samp: sampler;
@group(0) @binding(2) var<uniform> quad_u: QuadU;

@vertex fn vs_quad(@builtin(vertex_index) i: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[i];
    let r = quad_u.rect;
    var o: VsOut;
    // uv.y = 0 is the top edge (texture space).
    o.pos = vec4<f32>(mix(r.x, r.z, c.x), mix(r.w, r.y, c.y), 0.0, 1.0);
    o.uv = c;
    return o;
}

@fragment fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    let s = textureSample(quad_tex, quad_samp, in.uv);
    if (quad_u.mode == 1u) {
        return quad_u.tint * s.r;
    }
    return s;
}

fn srgb_eotf(v: vec3<f32>) -> vec3<f32> {
    let lo = v / 12.92;
    let hi = pow((v + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, v <= vec3<f32>(0.04045));
}

// Output: canvas -> Rec.709 code values (preview surface or RGBA8 readback).
@fragment fn fs_out(in: VsOut) -> @location(0) vec4<f32> {
    var code = to_code(textureSample(quad_tex, quad_samp, in.uv).rgb);
    if (quad_u.mode == 1u) {
        code = srgb_eotf(code);
    }
    return vec4<f32>(code, 1.0);
}

//#nv12
// Canvas -> NV12, BT.709 limited range, 4 bytes per u32. One invocation = a 4x2 pixel block:
// two luma words and one CbCr word (two chroma pairs, 2x2 box in code-value space).
struct Nv12U {
    width: u32,
    height: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var nv_src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> nv_out: array<u32>;
@group(0) @binding(2) var<uniform> nv_u: Nv12U;

fn code_at(x: u32, y: u32) -> vec3<f32> {
    return to_code(textureLoad(nv_src, vec2<u32>(x, y), 0).rgb);
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

fn byte(v: f32) -> u32 {
    return u32(clamp(floor(v + 0.5), 0.0, 255.0));
}

@compute @workgroup_size(8, 8) fn cs_nv12(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = nv_u.width;
    let h = nv_u.height;
    if (id.x * 4u >= w || id.y * 2u >= h) {
        return;
    }
    let x0 = id.x * 4u;
    let y0 = id.y * 2u;
    for (var row = 0u; row < 2u; row++) {
        var word = 0u;
        for (var i = 0u; i < 4u; i++) {
            let y = byte(16.0 + 219.0 * luma(code_at(x0 + i, y0 + row)));
            word = word | (y << (8u * i));
        }
        nv_out[((y0 + row) * w + x0) / 4u] = word;
    }
    var uv_word = 0u;
    for (var pair = 0u; pair < 2u; pair++) {
        let x = x0 + pair * 2u;
        let c = (code_at(x, y0) + code_at(x + 1u, y0) + code_at(x, y0 + 1u) + code_at(x + 1u, y0 + 1u)) * 0.25;
        let yl = luma(c);
        let cb = byte(128.0 + (c.b - yl) / 1.8556 * 224.0);
        let cr = byte(128.0 + (c.r - yl) / 1.5748 * 224.0);
        uv_word = uv_word | (cb << (16u * pair)) | (cr << (16u * pair + 8u));
    }
    nv_out[(w * h + id.y * w + x0) / 4u] = uv_word;
}
