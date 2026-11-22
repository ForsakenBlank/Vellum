//! a camera raw style develop panel in one node: white balance, tone, texture, clarity,
//! dehaze, vibrance, hsl per colour band, colour grading wheels, sharpening and noise
//! reduction, lens corrections, vignette and grain.
//!
//! it runs as up to four passes: lens corrections resample the picture, two blurs (a small
//! one for texture, sharpening and noise, a large one for clarity and dehaze) describe its
//! surroundings, and one kernel does everything else per pixel. white balance and exposure
//! work in linear light; tone, colour and grading work on a 2.2 gamma version of it, which
//! is close to how the eye spaces tones and keeps the sliders even.
//!
//! the lens centre and size are hidden parameters the image editor fills in from the
//! canvas, so corrections and vignettes stay centred on the picture, not on the layer.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::IRect;

use super::adjust::vibrance;
use super::colorfn::{
    clamp01, fbm, hsl_to_rgb, luma, mix, mix3, rgb_to_hsl, sample, smoothstep, value_noise,
};
use super::gaussian_blur::{blur_cpu, blur_gpu, weights};
use super::{premultiply, unpremultiply};

pub struct CameraRaw;

macro_rules! slider {
    ($id:literal, $label:literal) => {
        ParamSpec::float($id, $label, -100.0, 100.0, 0.0)
    };
}

pub const BANDS: [&str; 8] = [
    "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
];

static INFO: NodeInfo = NodeInfo {
    id: "effect.camera_raw",
    title: "Camera Raw",
    category: NodeCategory::Color,
    description: "Develops a photo: white balance, tone, presence, colour, detail, optics and effects.",
    inputs: ONE_INPUT,
    params: &[
        slider!("temperature", "Temperature"),
        slider!("tint", "Tint"),
        ParamSpec::float("exposure", "Exposure", -5.0, 5.0, 0.0),
        slider!("contrast", "Contrast"),
        slider!("highlights", "Highlights"),
        slider!("shadows", "Shadows"),
        slider!("whites", "Whites"),
        slider!("blacks", "Blacks"),
        slider!("texture", "Texture"),
        slider!("clarity", "Clarity"),
        slider!("dehaze", "Dehaze"),
        slider!("vibrance", "Vibrance"),
        slider!("saturation", "Saturation"),
        slider!("red_hue", "Red hue"),
        slider!("orange_hue", "Orange hue"),
        slider!("yellow_hue", "Yellow hue"),
        slider!("green_hue", "Green hue"),
        slider!("aqua_hue", "Aqua hue"),
        slider!("blue_hue", "Blue hue"),
        slider!("purple_hue", "Purple hue"),
        slider!("magenta_hue", "Magenta hue"),
        slider!("red_saturation", "Red saturation"),
        slider!("orange_saturation", "Orange saturation"),
        slider!("yellow_saturation", "Yellow saturation"),
        slider!("green_saturation", "Green saturation"),
        slider!("aqua_saturation", "Aqua saturation"),
        slider!("blue_saturation", "Blue saturation"),
        slider!("purple_saturation", "Purple saturation"),
        slider!("magenta_saturation", "Magenta saturation"),
        slider!("red_luminance", "Red luminance"),
        slider!("orange_luminance", "Orange luminance"),
        slider!("yellow_luminance", "Yellow luminance"),
        slider!("green_luminance", "Green luminance"),
        slider!("aqua_luminance", "Aqua luminance"),
        slider!("blue_luminance", "Blue luminance"),
        slider!("purple_luminance", "Purple luminance"),
        slider!("magenta_luminance", "Magenta luminance"),
        ParamSpec::degrees("shadows_hue", "Shadows hue", 0.0, 360.0, 220.0),
        ParamSpec::percent("shadows_sat", "Shadows saturation", 0.0, 100.0, 0.0),
        slider!("shadows_lum", "Shadows luminance"),
        ParamSpec::degrees("midtones_hue", "Midtones hue", 0.0, 360.0, 40.0),
        ParamSpec::percent("midtones_sat", "Midtones saturation", 0.0, 100.0, 0.0),
        slider!("midtones_lum", "Midtones luminance"),
        ParamSpec::degrees("highlights_hue", "Highlights hue", 0.0, 360.0, 45.0),
        ParamSpec::percent("highlights_sat", "Highlights saturation", 0.0, 100.0, 0.0),
        slider!("highlights_lum", "Highlights luminance"),
        ParamSpec::degrees("global_hue", "Global hue", 0.0, 360.0, 0.0),
        ParamSpec::percent("global_sat", "Global saturation", 0.0, 100.0, 0.0),
        slider!("global_lum", "Global luminance"),
        ParamSpec::percent("blending", "Blending", 0.0, 100.0, 50.0),
        slider!("balance", "Balance"),
        ParamSpec::float("sharpen_amount", "Sharpening", 0.0, 150.0, 0.0),
        ParamSpec::float("sharpen_radius", "Radius", 0.5, 3.0, 1.0),
        ParamSpec::percent("sharpen_masking", "Masking", 0.0, 100.0, 0.0),
        ParamSpec::percent("noise_luminance", "Noise reduction", 0.0, 100.0, 0.0),
        ParamSpec::percent("noise_color", "Color noise reduction", 0.0, 100.0, 0.0),
        slider!("distortion", "Distortion"),
        slider!("chromatic_aberration", "Chromatic aberration"),
        slider!("vignetting", "Lens vignetting"),
        slider!("vignette_amount", "Vignette"),
        ParamSpec::percent("vignette_midpoint", "Midpoint", 0.0, 100.0, 50.0),
        slider!("vignette_roundness", "Roundness"),
        ParamSpec::percent("vignette_feather", "Feather", 0.0, 100.0, 50.0),
        ParamSpec::percent("grain_amount", "Grain", 0.0, 100.0, 0.0),
        ParamSpec::percent("grain_size", "Grain size", 0.0, 100.0, 25.0),
        ParamSpec::percent("grain_roughness", "Grain roughness", 0.0, 100.0, 50.0),
        ParamSpec::point("center", "Lens centre", [0.0, 0.0]).hide(),
        ParamSpec::point("extent", "Half size", [0.0, 0.0]).hide(),
    ],
};

const LENS_KERNEL: Kernel = Kernel {
    name: "camera_raw_lens",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let g = param(16u);
    let l = param(19u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let radius = max(length(g.zw), 1.0);
    let d = (pos - g.xy) / radius;
    let k = 1.0 + l.x * dot(d, d);
    let base = g.xy + d * k * radius;
    let cr = sample0(g.xy + (base - g.xy) * (1.0 + l.y), true);
    let cg = sample0(base, true);
    let cb = sample0(g.xy + (base - g.xy) * (1.0 - l.y), true);
    // colour channels move apart; alpha follows green
    let ar = max(cr.a, 1e-6);
    let ab = max(cb.a, 1e-6);
    let ag = cg.a;
    return vec4<f32>(cr.r / ar * ag, cg.g, cb.b / ab * ag, ag);
}
"#,
};

const MAIN_KERNEL: Kernel = Kernel {
    name: "camera_raw",
    body: r#"
fn band_weight(h: f32, i: u32) -> f32 {
    var c = array<f32, 10>(-0.1666667, 0.0, 0.0833333, 0.1666667, 0.3333333, 0.5, 0.6666667, 0.75, 0.8333333, 1.0);
    let mid = c[i + 1u];
    let lo = c[i];
    let hi = c[i + 2u];
    var x = h;
    if (i == 0u && x > 0.9166667) { x = x - 1.0; }
    if (i == 7u && x < 0.0416667) { x = x + 1.0; }
    if (x <= lo || x >= hi) { return 0.0; }
    if (x <= mid) { return (x - lo) / (mid - lo); }
    return (hi - x) / (hi - mid);
}

fn grade(p: vec3<f32>, y: f32) -> vec3<f32> {
    let k = param(14u);
    let ws = pow(1.0 - smoothstep(0.0, k.x, y), k.y);
    let wh = pow(smoothstep(k.x, 1.0, y), k.y);
    let wm = max(1.0 - ws - wh, 0.0);
    let s = param(10u); let m = param(11u); let h = param(12u); let g = param(13u);
    return p + (ws * s.rgb + wm * m.rgb + wh * h.rgb + g.rgb) * 0.3
        + vec3<f32>(ws * s.w + wm * m.w + wh * h.w + g.w) * 0.3;
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let src = unpremultiply(load0(p));
    if (src.a <= 0.0) { return vec4<f32>(0.0); }
    let b1 = unpremultiply(load1(p)).rgb;
    let b2 = unpremultiply(load2(p)).rgb;
    let a1 = param(1u); let a2 = param(2u); let a3 = param(3u);
    let d14 = param(14u); let d15 = param(15u);
    var c = src.rgb;

    // detail
    let y0 = luma(c);
    let yb = luma(b1);
    let edge = 1.0 - smoothstep(0.02, 0.12, abs(y0 - yb));
    c = c + vec3<f32>((yb - y0) * d15.x * edge);
    let y1 = luma(c);
    c = vec3<f32>(y1) + mix(c - vec3<f32>(y0), b1 - vec3<f32>(yb), d15.y);
    let detail = c - b1;
    c = c + detail * a2.y * 0.8;
    let mid = 4.0 * clamp(y1, 0.0, 1.0) * (1.0 - clamp(y1, 0.0, 1.0));
    c = c + (c - b2) * a2.z * 0.5 * mid;
    let mask = smoothstep(0.0, d14.w * 0.1 + 1e-4, abs(luma(detail)));
    c = c + detail * d14.z * mask;
    if (a2.w > 0.0) {
        let dark = clamp(min(min(b2.r, b2.g), b2.b), 0.0, 1.0);
        let t = max(1.0 - a2.w * 0.95 * dark, 0.1);
        c = (c - vec3<f32>(1.0 - t)) / t;
    } else if (a2.w < 0.0) {
        c = mix(c, vec3<f32>(0.8), -a2.w * 0.5);
    }

    // white balance and exposure in linear light
    let g = param(16u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let radius = max(length(g.zw), 1.0);
    let rr = dot((pos - g.xy) / radius, (pos - g.xy) / radius);
    var lin = to_linear(c) * param(0u).rgb * param(0u).w * (1.0 + a3.w * rr);

    // tone on a 2.2 gamma version
    var t = pow(max(lin, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    if (a1.x > 0.0) {
        t = mix(t, smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), t), a1.x);
    } else {
        t = mix(t, vec3<f32>(0.5) + (t - vec3<f32>(0.5)) * 0.5, -a1.x);
    }
    let y = clamp(luma(t), 0.0, 1.0);
    let delta = smoothstep(0.5, 1.0, y) * a1.y * 0.35
        + (1.0 - smoothstep(0.0, 0.5, y)) * a1.z * 0.35
        + smoothstep(0.75, 1.0, y) * a1.w * 0.25
        + (1.0 - smoothstep(0.0, 0.25, y)) * a2.x * 0.25;
    t = t + vec3<f32>(delta);

    // colour
    let s = max(max(t.r, t.g), t.b) - min(min(t.r, t.g), t.b);
    var vib = a3.x;
    if (vib > 0.0) { vib = vib * (1.0 - clamp(s, 0.0, 1.0)); }
    let lv = luma(t);
    t = vec3<f32>(lv) + (t - vec3<f32>(lv)) * (1.0 + vib);
    t = vec3<f32>(lv) + (t - vec3<f32>(lv)) * (1.0 + a3.y);
    if (a3.z > 0.5) {
        var hsl = rgb_to_hsl(clamp01(t));
        var dh = 0.0; var ds = 0.0; var dl = 0.0;
        for (var i = 0u; i < 8u; i = i + 1u) {
            let w = band_weight(hsl.x, i);
            if (w <= 0.0) { continue; }
            dh = dh + w * param(4u + i / 4u)[i % 4u];
            ds = ds + w * param(6u + i / 4u)[i % 4u];
            dl = dl + w * param(8u + i / 4u)[i % 4u];
        }
        hsl.x = fract(hsl.x + dh * 0.0833333);
        let sat0 = hsl.y;
        hsl.y = clamp(hsl.y * (1.0 + ds), 0.0, 1.0);
        hsl.z = clamp(hsl.z + dl * 0.3 * sat0, 0.0, 1.0);
        t = hsl_to_rgb(hsl);
    }
    t = grade(t, clamp(luma(t), 0.0, 1.0));

    // effects
    let v = param(17u);
    if (v.x != 0.0) {
        let d = (pos - g.xy) / max(g.zw, vec2<f32>(1.0));
        let dist = mix(length(d), max(abs(d.x), abs(d.y)), clamp(-v.z, 0.0, 1.0) * 0.5) * mix(1.0, 0.85, clamp(v.z, 0.0, 1.0));
        let vm = smoothstep(v.y, v.y + v.w + 0.05, dist);
        if (v.x < 0.0) { t = t * (1.0 + v.x * vm); } else { t = mix(t, vec3<f32>(1.0), v.x * vm); }
    }
    let gr = param(18u);
    if (gr.x > 0.0) {
        let q = vec2<f32>(level0(p)) / gr.y;
        let n = mix(value_noise(q.x, q.y, 17u), fbm(q.x, q.y, 3u, 17u), gr.z) - 0.5;
        let yy = clamp(luma(t), 0.0, 1.0);
        t = t + vec3<f32>(n * gr.x * 0.12 * (4.0 * yy * (1.0 - yy) + 0.2));
    }

    var out = from_linear(pow(max(t, vec3<f32>(0.0)), vec3<f32>(2.2)));
    if (clamps()) { out = clamp01(out); }
    return premultiply(vec4<f32>(out, src.a));
}
"#,
};

/// everything the passes need, worked out once per render
struct Plan {
    params: Vec<[f32; 4]>,
    lens: bool,
    lens_reach: i32,
    small: Option<f32>,
    large: Option<f32>,
}

fn reach(sigma: Option<f32>) -> i32 {
    sigma.map_or(0, |s| ((weights(s).len() - 1) / 2) as i32)
}

fn tint(hue: f32, sat: f32) -> [f32; 3] {
    let c = hsl_to_rgb([hue / 360.0, 1.0, 0.5]);
    let l = luma(c);
    c.map(|v| (v - l) * sat)
}

fn plan(cx: &NodeCx) -> Option<Plan> {
    let p = cx.params;
    let f = |id: &str| p.float(id) / 100.0;
    let defaults_only = INFO.params.iter().filter(|s| !s.hidden).all(|spec| {
        cx.params
            .values
            .get(spec.id)
            .is_none_or(|v| spec.sanitize(v.clone()) == spec.default_value())
    });
    if defaults_only {
        return None;
    }
    let scale = cx.scale();
    let temp = f("temperature");
    let tint_v = f("tint");
    let wb = [1.0 + 0.3 * temp, 1.0 - 0.3 * tint_v, 1.0 - 0.3 * temp];
    let hsl_on = BANDS.iter().any(|b| {
        ["hue", "saturation", "luminance"]
            .iter()
            .any(|k| p.float(&format!("{b}_{k}")) != 0.0)
    });
    let band = |k: &str, half: usize| {
        let mut v = [0.0; 4];
        for (i, b) in BANDS[half * 4..half * 4 + 4].iter().enumerate() {
            v[i] = p.float(&format!("{b}_{k}")) / 100.0;
        }
        v
    };
    let wheel = |name: &str| {
        let t = tint(p.float(&format!("{name}_hue")), f(&format!("{name}_sat")));
        [t[0], t[1], t[2], f(&format!("{name}_lum"))]
    };
    let blend = f("blending");
    let split = (0.5 - f("balance") * 0.3).clamp(0.1, 0.9);
    let center = p.point("center");
    let extent = p.point("extent");
    let distortion = f("distortion") * 0.25;
    let ca = f("chromatic_aberration") * 0.004;
    let lens = distortion != 0.0 || ca != 0.0;
    let radius = (extent[0].hypot(extent[1]) * scale).max(1.0);
    let lens_reach = if lens {
        (radius * (distortion.abs() * 2.0 + ca.abs() * 2.0)).ceil() as i32 + 2
    } else {
        0
    };
    let sharpen = p.float("sharpen_amount") / 100.0 * 1.5;
    let (nl, nc) = (f("noise_luminance"), f("noise_color"));
    let small_needed = f("texture") != 0.0 || sharpen != 0.0 || nl != 0.0 || nc != 0.0;
    let large_needed = f("clarity") != 0.0 || f("dehaze") != 0.0;
    let small = small_needed.then(|| (p.float("sharpen_radius") * scale).max(0.3));
    let large = large_needed.then(|| (25.0 * scale).max(0.3));
    let grain_size = 1.0 + f("grain_size") * 4.0;
    let params = vec![
        [wb[0], wb[1], wb[2], 2f32.powf(p.float("exposure"))],
        [f("contrast"), f("highlights"), f("shadows"), f("whites")],
        [f("blacks"), f("texture"), f("clarity"), f("dehaze")],
        [
            f("vibrance"),
            f("saturation"),
            f32::from(u8::from(hsl_on)),
            f("vignetting"),
        ],
        band("hue", 0),
        band("hue", 1),
        band("saturation", 0),
        band("saturation", 1),
        band("luminance", 0),
        band("luminance", 1),
        wheel("shadows"),
        wheel("midtones"),
        wheel("highlights"),
        wheel("global"),
        [split, mix(2.0, 0.5, blend), sharpen, f("sharpen_masking")],
        [nl, nc, 0.0, 0.0],
        [
            center[0] * scale,
            center[1] * scale,
            extent[0] * scale,
            extent[1] * scale,
        ],
        [
            f("vignette_amount"),
            f("vignette_midpoint"),
            f("vignette_roundness"),
            f("vignette_feather"),
        ],
        [f("grain_amount"), grain_size, f("grain_roughness"), 0.0],
        [distortion, ca, 0.0, 0.0],
    ];
    Some(Plan {
        params,
        lens,
        lens_reach,
        small,
        large,
    })
}

fn band_weight(h: f32, i: usize) -> f32 {
    const C: [f32; 10] = [
        -0.166_666_7,
        0.0,
        0.083_333_3,
        0.166_666_7,
        0.333_333_3,
        0.5,
        0.666_666_7,
        0.75,
        0.833_333_3,
        1.0,
    ];
    let (lo, mid, hi) = (C[i], C[i + 1], C[i + 2]);
    let mut x = h;
    if i == 0 && x > 0.916_666_7 {
        x -= 1.0;
    }
    if i == 7 && x < 0.041_666_7 {
        x += 1.0;
    }
    if x <= lo || x >= hi {
        0.0
    } else if x <= mid {
        (x - lo) / (mid - lo)
    } else {
        (hi - x) / (hi - mid)
    }
}

fn lens_pixel(pl: &Plan, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
    let g = pl.params[16];
    let l = pl.params[19];
    let pos = [x as f32 + 0.5, y as f32 + 0.5];
    let radius = g[2].hypot(g[3]).max(1.0);
    let d = [(pos[0] - g[0]) / radius, (pos[1] - g[1]) / radius];
    let k = 1.0 + l[0] * (d[0] * d[0] + d[1] * d[1]);
    let base = [g[0] + d[0] * k * radius, g[1] + d[1] * k * radius];
    let at = |s: f32| {
        sample(
            input,
            [g[0] + (base[0] - g[0]) * s, g[1] + (base[1] - g[1]) * s],
            true,
        )
    };
    let (cr, cg, cb) = (at(1.0 + l[1]), sample(input, base, true), at(1.0 - l[1]));
    let ag = cg[3];
    [
        cr[0] / cr[3].max(1e-6) * ag,
        cg[1],
        cb[2] / cb[3].max(1e-6) * ag,
        ag,
    ]
}

fn smooth3(e0: f32, e1: f32, c: [f32; 3]) -> [f32; 3] {
    c.map(|v| smoothstep(e0, e1, v))
}

fn main_pixel(
    pl: &Plan,
    cx: &NodeCx,
    src: [f32; 4],
    b1: [f32; 4],
    b2: [f32; 4],
    x: i32,
    y: i32,
) -> [f32; 4] {
    let src = unpremultiply(src);
    if src[3] <= 0.0 {
        return [0.0; 4];
    }
    let b1 = unpremultiply(b1);
    let b2 = unpremultiply(b2);
    let pr = &pl.params;
    let (a1, a2, a3, d14, d15) = (pr[1], pr[2], pr[3], pr[14], pr[15]);
    let b1 = [b1[0], b1[1], b1[2]];
    let b2 = [b2[0], b2[1], b2[2]];
    let mut c = [src[0], src[1], src[2]];

    let y0 = luma(c);
    let yb = luma(b1);
    let edge = 1.0 - smoothstep(0.02, 0.12, (y0 - yb).abs());
    c = c.map(|v| v + (yb - y0) * d15[0] * edge);
    let y1 = luma(c);
    for k in 0..3 {
        c[k] = y1 + mix(c[k] - y0, b1[k] - yb, d15[1]);
    }
    let detail = [c[0] - b1[0], c[1] - b1[1], c[2] - b1[2]];
    for k in 0..3 {
        c[k] += detail[k] * a2[1] * 0.8;
    }
    let yc = y1.clamp(0.0, 1.0);
    let mid = 4.0 * yc * (1.0 - yc);
    for k in 0..3 {
        c[k] += (c[k] - b2[k]) * a2[2] * 0.5 * mid;
    }
    let mask = smoothstep(0.0, d14[3] * 0.1 + 1e-4, luma(detail).abs());
    for k in 0..3 {
        c[k] += detail[k] * d14[2] * mask;
    }
    if a2[3] > 0.0 {
        let dark = b2[0].min(b2[1]).min(b2[2]).clamp(0.0, 1.0);
        let t = (1.0 - a2[3] * 0.95 * dark).max(0.1);
        c = c.map(|v| (v - (1.0 - t)) / t);
    } else if a2[3] < 0.0 {
        c = c.map(|v| mix(v, 0.8, -a2[3] * 0.5));
    }

    let g = pr[16];
    let pos = [x as f32 + 0.5, y as f32 + 0.5];
    let radius = g[2].hypot(g[3]).max(1.0);
    let (dx, dy) = ((pos[0] - g[0]) / radius, (pos[1] - g[1]) / radius);
    let rr = dx * dx + dy * dy;
    let enc = cx.working.encoding;
    let p0 = pr[0];
    let lin = [0, 1, 2].map(|k| enc.decode(c[k]) * p0[k] * p0[3] * (1.0 + a3[3] * rr));

    let mut t = lin.map(|v| v.max(0.0).powf(1.0 / 2.2));
    if a1[0] > 0.0 {
        t = mix3(t, smooth3(0.0, 1.0, t), a1[0]);
    } else {
        t = mix3(t, t.map(|v| 0.5 + (v - 0.5) * 0.5), -a1[0]);
    }
    let yy = luma(t).clamp(0.0, 1.0);
    let delta = smoothstep(0.5, 1.0, yy) * a1[1] * 0.35
        + (1.0 - smoothstep(0.0, 0.5, yy)) * a1[2] * 0.35
        + smoothstep(0.75, 1.0, yy) * a1[3] * 0.25
        + (1.0 - smoothstep(0.0, 0.25, yy)) * a2[0] * 0.25;
    t = t.map(|v| v + delta);

    let v4 = vibrance([t[0], t[1], t[2], 1.0], a3[0], a3[1]);
    t = [v4[0], v4[1], v4[2]];
    if a3[2] > 0.5 {
        let mut hsl = rgb_to_hsl(clamp01(t));
        let (mut dh, mut ds, mut dl) = (0.0, 0.0, 0.0);
        for i in 0..8 {
            let w = band_weight(hsl[0], i);
            if w <= 0.0 {
                continue;
            }
            dh += w * pr[4 + i / 4][i % 4];
            ds += w * pr[6 + i / 4][i % 4];
            dl += w * pr[8 + i / 4][i % 4];
        }
        let h = hsl[0] + dh * 0.083_333_3;
        hsl[0] = h - h.floor();
        let sat0 = hsl[1];
        hsl[1] = (hsl[1] * (1.0 + ds)).clamp(0.0, 1.0);
        hsl[2] = (hsl[2] + dl * 0.3 * sat0).clamp(0.0, 1.0);
        t = hsl_to_rgb(hsl);
    }
    let yg = luma(t).clamp(0.0, 1.0);
    let ws = (1.0 - smoothstep(0.0, d14[0], yg)).powf(d14[1]);
    let wh = smoothstep(d14[0], 1.0, yg).powf(d14[1]);
    let wm = (1.0 - ws - wh).max(0.0);
    let (s, m, h, gl) = (pr[10], pr[11], pr[12], pr[13]);
    for k in 0..3 {
        t[k] += (ws * s[k] + wm * m[k] + wh * h[k] + gl[k]) * 0.3
            + (ws * s[3] + wm * m[3] + wh * h[3] + gl[3]) * 0.3;
    }

    let v = pr[17];
    if v[0] != 0.0 {
        let d = [
            (pos[0] - g[0]) / g[2].max(1.0),
            (pos[1] - g[1]) / g[3].max(1.0),
        ];
        let dist = mix(
            d[0].hypot(d[1]),
            d[0].abs().max(d[1].abs()),
            (-v[2]).clamp(0.0, 1.0) * 0.5,
        ) * mix(1.0, 0.85, v[2].clamp(0.0, 1.0));
        let vm = smoothstep(v[1], v[1] + v[3] + 0.05, dist);
        t = if v[0] < 0.0 {
            t.map(|c| c * (1.0 + v[0] * vm))
        } else {
            t.map(|c| mix(c, 1.0, v[0] * vm))
        };
    }
    let gr = pr[18];
    if gr[0] > 0.0 {
        let s0 = 1i32 << cx.level.min(30);
        let q = [(x * s0) as f32 / gr[1], (y * s0) as f32 / gr[1]];
        let n = mix(value_noise(q[0], q[1], 17), fbm(q[0], q[1], 3, 17), gr[2]) - 0.5;
        let yy = luma(t).clamp(0.0, 1.0);
        let add = n * gr[0] * 0.12 * (4.0 * yy * (1.0 - yy) + 0.2);
        t = t.map(|c| c + add);
    }
    let mut out = t.map(|v| enc.encode(v.max(0.0).powf(2.2)));
    if cx.clamp() {
        out = clamp01(out);
    }
    premultiply([out[0], out[1], out[2], src[3]])
}

impl NodeKind for CameraRaw {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        match plan(cx) {
            Some(pl) => region.expand(pl.lens_reach + reach(pl.small).max(reach(pl.large))),
            None => region,
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let Some(input) = inputs.first().copied().flatten() else {
            return Out::Empty;
        };
        let Some(pl) = plan(cx) else {
            return Out::Input(0);
        };
        let around = region.expand(reach(pl.small).max(reach(pl.large)));
        let corrected;
        let src = if pl.lens {
            corrected = CpuImage::from_fn(around, |x, y| lens_pixel(&pl, input, x, y));
            &corrected
        } else {
            input
        };
        let small = pl.small.map(|s| blur_cpu(src, region, s, true));
        let large = pl.large.map(|s| blur_cpu(src, region, s, true));
        Out::New(CpuImage::from_fn(region, |x, y| {
            let c = src.get(x, y);
            let b1 = small.as_ref().map_or(c, |b| b.get(x, y));
            let b2 = large.as_ref().map_or(c, |b| b.get(x, y));
            main_pixel(&pl, cx, c, b1, b2, x, y)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        let Some(pl) = plan(cx) else {
            return Ok(Out::Input(0));
        };
        let around = region.expand(reach(pl.small).max(reach(pl.large)));
        fn d(params: &[[f32; 4]]) -> Dispatch<'_> {
            Dispatch {
                params,
                ..Default::default()
            }
        }
        let corrected;
        let src = if pl.lens {
            corrected = gx.dispatch(&LENS_KERNEL, &[Some(input)], around, d(&pl.params))?;
            &corrected
        } else {
            input
        };
        let small = match pl.small {
            Some(s) => Some(blur_gpu(gx, src, region, s, true)?),
            None => None,
        };
        let large = match pl.large {
            Some(s) => Some(blur_gpu(gx, src, region, s, true)?),
            None => None,
        };
        Ok(Out::New(gx.dispatch(
            &MAIN_KERNEL,
            &[
                Some(src),
                Some(small.as_ref().unwrap_or(src)),
                Some(large.as_ref().unwrap_or(src)),
            ],
            region,
            d(&pl.params),
        )?))
    }
}
