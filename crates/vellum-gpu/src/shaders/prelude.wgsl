// shared by every kernel. a kernel file adds `fn kernel(p: vec2<i32>) -> vec4<f32>`, where
// p is the absolute pixel being written at the current level. colours are premultiplied.

struct Frame {
    out_rect: vec4<i32>,
    in_rect: array<vec4<i32>, 4>,
    level: u32,
    encoding: u32,
    gamma: f32,
    data_len: u32,
    params: array<vec4<f32>, 32>,
}

@group(0) @binding(0) var out_tex: texture_storage_2d<{{FORMAT}}, write>;
@group(0) @binding(1) var<uniform> F: Frame;
@group(0) @binding(2) var in0: texture_2d<f32>;
@group(0) @binding(3) var in1: texture_2d<f32>;
@group(0) @binding(4) var in2: texture_2d<f32>;
@group(0) @binding(5) var in3: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> data: array<f32>;
@group(0) @binding(7) var lut: texture_3d<f32>;

fn param(i: u32) -> vec4<f32> {
    return F.params[i];
}

fn inside(r: vec4<i32>, p: vec2<i32>) -> bool {
    return p.x >= r.x && p.y >= r.y && p.x < r.x + r.z && p.y < r.y + r.w;
}

fn clamp_to(r: vec4<i32>, p: vec2<i32>) -> vec2<i32> {
    return clamp(p - r.xy, vec2<i32>(0), max(r.zw - vec2<i32>(1), vec2<i32>(0)));
}

// loads that read transparent outside their input's rectangle
fn load0(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[0];
    if (!inside(r, p)) { return vec4<f32>(0.0); }
    return textureLoad(in0, p - r.xy, 0);
}
fn load1(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[1];
    if (!inside(r, p)) { return vec4<f32>(0.0); }
    return textureLoad(in1, p - r.xy, 0);
}
fn load2(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[2];
    if (!inside(r, p)) { return vec4<f32>(0.0); }
    return textureLoad(in2, p - r.xy, 0);
}
fn load3(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[3];
    if (!inside(r, p)) { return vec4<f32>(0.0); }
    return textureLoad(in3, p - r.xy, 0);
}

// a load that repeats the nearest edge pixel, for sampling filters
fn load0_clamped(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[0];
    if (r.z <= 0 || r.w <= 0) { return vec4<f32>(0.0); }
    return textureLoad(in0, clamp_to(r, p), 0);
}

fn unpremultiply(c: vec4<f32>) -> vec4<f32> {
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    return vec4<f32>(c.rgb / c.a, c.a);
}

fn premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// transfer functions, ids match `Transfer::id` in color.rs
fn decode1(id: u32, g: f32, v: f32) -> f32 {
    let s = sign(v);
    let a = abs(v);
    switch id {
        case 1u: {
            if (a <= 0.04045) { return s * a / 12.92; }
            return s * pow((a + 0.055) / 1.055, 2.4);
        }
        case 2u: { return s * pow(a, g); }
        case 3u: {
            if (a < 0.081) { return s * a / 4.5; }
            return s * pow((a + 0.099) / 1.099, 1.0 / 0.45);
        }
        case 4u: {
            let m1 = 2610.0 / 16384.0;
            let m2 = 2523.0 / 4096.0 * 128.0;
            let c1 = 3424.0 / 4096.0;
            let c2 = 2413.0 / 4096.0 * 32.0;
            let c3 = 2392.0 / 4096.0 * 32.0;
            let p = pow(max(v, 0.0), 1.0 / m2);
            let n = max(p - c1, 0.0) / (c2 - c3 * p);
            return pow(n, 1.0 / m1) * 10000.0 / 203.0;
        }
        case 5u: {
            let x = max(v, 0.0);
            var e: f32;
            if (x <= 0.5) { e = x * x / 3.0; } else { e = (exp((x - 0.5599107) / 0.17883277) + 0.28466892) / 12.0; }
            return e / 0.265498;
        }
        case 6u: {
            if (v <= 0.15525114) { return (v - 0.07290553) / 10.540238; }
            if (v < 1.468) { return exp2(v * 17.52 - 9.72); }
            return 65504.0;
        }
        default: { return v; }
    }
}

fn encode1(id: u32, g: f32, l: f32) -> f32 {
    let s = sign(l);
    let a = abs(l);
    switch id {
        case 1u: {
            if (a <= 0.0031308) { return s * a * 12.92; }
            return s * (1.055 * pow(a, 1.0 / 2.4) - 0.055);
        }
        case 2u: { return s * pow(a, 1.0 / g); }
        case 3u: {
            if (a < 0.018) { return s * a * 4.5; }
            return s * (1.099 * pow(a, 0.45) - 0.099);
        }
        case 4u: {
            let m1 = 2610.0 / 16384.0;
            let m2 = 2523.0 / 4096.0 * 128.0;
            let c1 = 3424.0 / 4096.0;
            let c2 = 2413.0 / 4096.0 * 32.0;
            let c3 = 2392.0 / 4096.0 * 32.0;
            let y = pow(max(l, 0.0) * 203.0 / 10000.0, m1);
            return pow((c1 + c2 * y) / (1.0 + c3 * y), m2);
        }
        case 5u: {
            let e = min(max(l, 0.0) * 0.265498, 1.0);
            if (e <= 1.0 / 12.0) { return sqrt(3.0 * e); }
            return 0.17883277 * log(12.0 * e - 0.28466892) + 0.5599107;
        }
        case 6u: {
            if (l <= 0.0078125) { return 10.540238 * l + 0.07290553; }
            return (log2(l) + 9.72) / 17.52;
        }
        default: { return l; }
    }
}

fn decode3(id: u32, g: f32, c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(decode1(id, g, c.r), decode1(id, g, c.g), decode1(id, g, c.b));
}

fn encode3(id: u32, g: f32, c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(encode1(id, g, c.r), encode1(id, g, c.g), encode1(id, g, c.b));
}

// working space values to linear light and back
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return decode3(F.encoding, F.gamma, c);
}

fn from_linear(c: vec3<f32>) -> vec3<f32> {
    return encode3(F.encoding, F.gamma, c);
}

fn lut_at(i: vec3<i32>) -> vec3<f32> {
    return textureLoad(lut, i, 0).rgb;
}

fn apply_lut(c: vec3<f32>) -> vec3<f32> {
    let n = i32(textureDimensions(lut).x);
    let m = f32(n - 1);
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * m;
    let f = clamp(floor(x), vec3<f32>(0.0), vec3<f32>(max(m - 1.0, 0.0)));
    let i0 = vec3<i32>(f);
    let i1 = min(i0 + vec3<i32>(1), vec3<i32>(n - 1));
    let t = x - f;
    let c00 = mix(lut_at(vec3<i32>(i0.x, i0.y, i0.z)), lut_at(vec3<i32>(i1.x, i0.y, i0.z)), t.x);
    let c10 = mix(lut_at(vec3<i32>(i0.x, i1.y, i0.z)), lut_at(vec3<i32>(i1.x, i1.y, i0.z)), t.x);
    let c01 = mix(lut_at(vec3<i32>(i0.x, i0.y, i1.z)), lut_at(vec3<i32>(i1.x, i0.y, i1.z)), t.x);
    let c11 = mix(lut_at(vec3<i32>(i0.x, i1.y, i1.z)), lut_at(vec3<i32>(i1.x, i1.y, i1.z)), t.x);
    return mix(mix(c00, c10, t.y), mix(c01, c11, t.y), t.z);
}

// runs a colour transform packed by `ColorTransform::pack`, starting at params[base]
fn color_steps(base: u32, rgb: vec3<f32>) -> vec3<f32> {
    var c = rgb;
    let count = u32(param(base).x);
    for (var i = 0u; i < count; i = i + 1u) {
        let at = base + 1u + i * 4u;
        let head = param(at);
        let kind = u32(head.x);
        let id = u32(head.y);
        switch kind {
            case 1u: { c = decode3(id, head.z, c); }
            case 2u: { c = encode3(id, head.z, c); }
            case 3u: {
                let r0 = param(at + 1u).xyz;
                let r1 = param(at + 2u).xyz;
                let r2 = param(at + 3u).xyz;
                c = vec3<f32>(dot(r0, c), dot(r1, c), dot(r2, c));
            }
            case 4u: { c = apply_lut(c); }
            case 5u: { c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
            default: {}
        }
    }
    return c;
}

// blend modes, mirroring blend.rs. ids follow `BlendMode::ALL`.
fn hard_light(b: f32, s: f32) -> f32 {
    if (s <= 0.5) { return 2.0 * b * s; }
    return 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
}

fn color_burn(b: f32, s: f32) -> f32 {
    if (b >= 1.0) { return 1.0; }
    if (s <= 0.0) { return 0.0; }
    return 1.0 - min((1.0 - b) / s, 1.0);
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if (b <= 0.0) { return 0.0; }
    if (s >= 1.0) { return 1.0; }
    return min(b / (1.0 - s), 1.0);
}

fn blend_channel(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 2u: { return min(b, s); }
        case 3u: { return b * s; }
        case 4u: { return color_burn(b, s); }
        case 5u: { return max(b + s - 1.0, 0.0); }
        case 7u: { return max(b, s); }
        case 8u: { return b + s - b * s; }
        case 9u: { return color_dodge(b, s); }
        case 10u: { return b + s; }
        case 12u: { return hard_light(s, b); }
        case 13u: {
            if (s <= 0.5) { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
            return b + (2.0 * s - 1.0) * (sqrt(max(b, 0.0)) - b);
        }
        case 14u: { return hard_light(b, s); }
        case 15u: {
            if (s <= 0.5) { return color_burn(b, 2.0 * s); }
            return color_dodge(b, 2.0 * (s - 0.5));
        }
        case 16u: { return b + 2.0 * s - 1.0; }
        case 17u: {
            if (s <= 0.5) { return min(b, 2.0 * s); }
            return max(b, 2.0 * s - 1.0);
        }
        case 18u: {
            if (b + s >= 1.0) { return 1.0; }
            return 0.0;
        }
        case 19u: { return abs(b - s); }
        case 20u: { return b + s - 2.0 * b * s; }
        case 21u: { return max(b - s, 0.0); }
        case 22u: {
            if (s <= 0.0) {
                if (b > 0.0) { return 1.0; }
                return 0.0;
            }
            return b / s;
        }
        default: { return s; }
    }
}

fn blend_lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn clip_color(c_in: vec3<f32>, clamp_top: bool) -> vec3<f32> {
    var c = c_in;
    let l = blend_lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    if (n < 0.0 && l - n > 1e-9) { c = l + (c - l) * l / (l - n); }
    if (clamp_top && x > 1.0 && x - l > 1e-9) { c = l + (c - l) * (1.0 - l) / (x - l); }
    return c;
}

fn set_lum(c: vec3<f32>, l: f32, clamp_top: bool) -> vec3<f32> {
    return clip_color(c + (l - blend_lum(c)), clamp_top);
}

fn blend_sat(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    if (mx - mn <= 1e-9) { return vec3<f32>(0.0); }
    return (c - mn) * s / (mx - mn);
}

fn blend_color(mode: u32, b: vec3<f32>, s: vec3<f32>, clamp_out: bool) -> vec3<f32> {
    var out: vec3<f32>;
    switch mode {
        case 6u: {
            if (s.r + s.g + s.b < b.r + b.g + b.b) { out = s; } else { out = b; }
        }
        case 11u: {
            if (s.r + s.g + s.b > b.r + b.g + b.b) { out = s; } else { out = b; }
        }
        case 23u: { out = set_lum(set_sat(s, blend_sat(b)), blend_lum(b), clamp_out); }
        case 24u: { out = set_lum(set_sat(b, blend_sat(s)), blend_lum(b), clamp_out); }
        case 25u: { out = set_lum(s, blend_lum(b), clamp_out); }
        case 26u: { out = set_lum(b, blend_lum(s), clamp_out); }
        default: {
            out = vec3<f32>(blend_channel(mode, b.r, s.r), blend_channel(mode, b.g, s.g), blend_channel(mode, b.b, s.b));
        }
    }
    if (clamp_out) { out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)); }
    return out;
}

fn hash_u32(v_in: u32) -> u32 {
    let v = v_in * 747796405u + 2891336453u;
    let w = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (w >> 22u) ^ w;
}

fn pixel_noise(x: i32, y: i32) -> f32 {
    return f32(hash_u32(bitcast<u32>(x) ^ hash_u32(bitcast<u32>(y)))) / 4294967296.0;
}

// the level 0 pixel a level pixel starts at, for things that must not move with zoom
fn level0(p: vec2<i32>) -> vec2<i32> {
    return p * (1 << F.level);
}

// source over with a blend mode, the same maths as `blend::composite`
fn composite(mode: u32, backdrop: vec4<f32>, layer: vec4<f32>, alpha: f32, clamp_out: bool, p0: vec2<i32>) -> vec4<f32> {
    var sa = layer.a * alpha;
    if (mode == 1u) {
        if (pixel_noise(p0.x, p0.y) < sa) { sa = 1.0; } else { sa = 0.0; }
    }
    if (sa <= 0.0) { return backdrop; }
    let ba = backdrop.a;
    var cs = vec3<f32>(0.0);
    if (layer.a > 0.0) { cs = layer.rgb / layer.a; }
    var mixed = cs;
    if (ba > 0.0) {
        let cb = backdrop.rgb / ba;
        let b = blend_color(mode, cb, cs, clamp_out);
        mixed = (1.0 - ba) * cs + ba * b;
    }
    return vec4<f32>(sa * mixed + (1.0 - sa) * backdrop.rgb, sa + ba * (1.0 - sa));
}


// colours are kept in [0, 1] in gamma encoded working spaces (linear is encoding 0)
fn clamps() -> bool {
    return F.encoding != 0u;
}

fn clamp01(c: vec3<f32>) -> vec3<f32> {
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// photoshop's luminosity weights, the same as the blend modes use
fn luma(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn rgb_to_hsl(c: vec3<f32>) -> vec3<f32> {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    let l = (mx + mn) * 0.5;
    let d = mx - mn;
    if (d <= 1e-7) { return vec3<f32>(0.0, 0.0, l); }
    var s: f32;
    if (l < 0.5) { s = d / (mx + mn); } else { s = d / max(2.0 - mx - mn, 1e-7); }
    var h: f32;
    if (mx == c.r) {
        h = (c.g - c.b) / d;
        if (h < 0.0) { h = h + 6.0; }
    } else if (mx == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3<f32>(h / 6.0, s, l);
}

fn hue_part(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in - floor(t_in);
    if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
    if (t < 0.5) { return q; }
    if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb(c: vec3<f32>) -> vec3<f32> {
    if (c.y <= 0.0) { return vec3<f32>(c.z); }
    var q: f32;
    if (c.z < 0.5) { q = c.z * (1.0 + c.y); } else { q = c.z + c.y - c.z * c.y; }
    let p = 2.0 * c.z - q;
    return vec3<f32>(hue_part(p, q, c.x + 1.0 / 3.0), hue_part(p, q, c.x), hue_part(p, q, c.x - 1.0 / 3.0));
}


// bilinear sample of input 0 at a position in level pixels, pixel centres at +0.5.
// `edge` repeats the nearest edge pixel instead of reading transparent.
fn sample0(pos: vec2<f32>, edge: bool) -> vec4<f32> {
    let f = pos - vec2<f32>(0.5);
    let i = vec2<i32>(floor(f));
    let t = f - floor(f);
    var a: vec4<f32>; var b: vec4<f32>; var c: vec4<f32>; var d: vec4<f32>;
    if (edge) {
        a = load0_clamped(i);
        b = load0_clamped(i + vec2<i32>(1, 0));
        c = load0_clamped(i + vec2<i32>(0, 1));
        d = load0_clamped(i + vec2<i32>(1, 1));
    } else {
        a = load0(i);
        b = load0(i + vec2<i32>(1, 0));
        c = load0(i + vec2<i32>(0, 1));
        d = load0(i + vec2<i32>(1, 1));
    }
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

// smooth value noise in [0, 1] on a grid of `cell` units, the same as `colorfn::value_noise`
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let ix = i32(floor(x));
    let iy = i32(floor(y));
    let tx = x - floor(x);
    let ty = y - floor(y);
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let s = i32(seed * 7919u);
    let a = pixel_noise(ix + s, iy);
    let b = pixel_noise(ix + 1 + s, iy);
    let c = pixel_noise(ix + s, iy + 1);
    let d = pixel_noise(ix + 1 + s, iy + 1);
    return mix(mix(a, b, sx), mix(c, d, sx), sy);
}

// a few octaves of value noise, the same as `colorfn::fbm`
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var norm = 0.0;
    var fx = x;
    var fy = y;
    for (var i = 0u; i < octaves; i = i + 1u) {
        sum = sum + amp * value_noise(fx, fy, seed + i);
        norm = norm + amp;
        amp = amp * 0.5;
        fx = fx * 2.0;
        fy = fy * 2.0;
    }
    return sum / max(norm, 1e-6);
}

// samples a table of `n` entries per channel from the data buffer, linear between entries
fn table_at(base: u32, n: u32, v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0) * f32(n - 1u);
    let i = min(u32(floor(x)), n - 2u);
    let t = x - f32(i);
    var r = mix(data[base + i], data[base + i + 1u], t);
    if (v > 1.0) { r = r + (v - 1.0); }
    if (v < 0.0) { r = r + v; }
    return r;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u32(F.out_rect.z) || gid.y >= u32(F.out_rect.w)) { return; }
    let p = F.out_rect.xy + vec2<i32>(gid.xy);
    textureStore(out_tex, vec2<i32>(gid.xy), kernel(p));
}
