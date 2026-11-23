//! the colour adjustments from photoshop's image, adjustments menu that look at one pixel
//! at a time. each is a [`Simple`] effect with a `point_kernel!` and the same maths on the
//! cpu in `pixel`. values are on photoshop's scales (0 to 255, percent, degrees) and the
//! maths runs on straight colour in the working space's encoding, like levels.
//!
//! curves, brightness and contrast, invert and posterize all bake a table per channel and
//! share one kernel, which keeps the cpu and gpu answers identical by construction.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, GradientStop, ParamSpec};
use crate::point_kernel;

use super::colorfn::{
    clamp01, hsl_to_rgb, luma, mix, mix3, rgb_to_hsl, set_lum, smoothstep, table_at, working_color,
};
use super::simple::{Setup, Simple, point};

/// entries per channel in baked tone tables
pub const TABLE: usize = 1024;

/// applies three baked tables, one per channel, from the data buffer
const TABLE_KERNEL: Kernel = point_kernel!(
    "tone_table",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let n = 1024u;
    return vec4<f32>(table_at(0u, n, c.r), table_at(n, n, c.g), table_at(2u * n, n, c.b), c.a);
}
"#
);

fn table_pixel(s: &Setup, cx: &NodeCx, c: [f32; 4]) -> [f32; 4] {
    point(cx, c, |c| {
        [
            table_at(&s.data, 0, TABLE, c[0]),
            table_at(&s.data, TABLE, TABLE, c[1]),
            table_at(&s.data, 2 * TABLE, TABLE, c[2]),
            c[3],
        ]
    })
}

/// three tables from one function per channel
fn bake(f: impl Fn(usize, f32) -> f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(3 * TABLE);
    for ch in 0..3 {
        for i in 0..TABLE {
            out.push(f(ch, i as f32 / (TABLE - 1) as f32));
        }
    }
    out
}

/// a natural cubic spline through the points, like photoshop's curves, evaluated at x
/// and kept in [0, 1]
pub fn spline(points: &[[f32; 2]], x: f32) -> f32 {
    let n = points.len();
    if n == 0 {
        return x;
    }
    if n == 1 {
        return points[0][1];
    }
    if x <= points[0][0] {
        return points[0][1];
    }
    if x >= points[n - 1][0] {
        return points[n - 1][1];
    }
    let m = spline_slopes(points);
    let mut i = 0;
    while i + 2 < n && x > points[i + 1][0] {
        i += 1;
    }
    let (x0, y0) = (points[i][0], points[i][1]);
    let (x1, y1) = (points[i + 1][0], points[i + 1][1]);
    let h = (x1 - x0).max(1e-6);
    let a = (x1 - x) / h;
    let b = (x - x0) / h;
    let y = a * y0 + b * y1 + ((a * a * a - a) * m[i] + (b * b * b - b) * m[i + 1]) * h * h / 6.0;
    y.clamp(0.0, 1.0)
}

/// second derivatives of a natural cubic spline, by the tridiagonal algorithm
fn spline_slopes(p: &[[f32; 2]]) -> Vec<f32> {
    let n = p.len();
    let mut m = vec![0.0f32; n];
    if n < 3 {
        return m;
    }
    let mut u = vec![0.0f32; n];
    for i in 1..n - 1 {
        let sig = (p[i][0] - p[i - 1][0]) / (p[i + 1][0] - p[i - 1][0]).max(1e-6);
        let q = sig * m[i - 1] + 2.0;
        m[i] = (sig - 1.0) / q;
        let d = (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0]).max(1e-6)
            - (p[i][1] - p[i - 1][1]) / (p[i][0] - p[i - 1][0]).max(1e-6);
        u[i] = (6.0 * d / (p[i + 1][0] - p[i - 1][0]).max(1e-6) - sig * u[i - 1]) / q;
    }
    m[n - 1] = 0.0;
    for i in (0..n - 1).rev() {
        m[i] = m[i] * m[i + 1] + u[i];
    }
    m
}

fn is_identity_curve(p: &[[f32; 2]]) -> bool {
    p.len() == 2 && p[0] == [0.0, 0.0] && p[1] == [1.0, 1.0]
}

// ---- curves ----

pub struct Curves;

static CURVES: NodeInfo = NodeInfo {
    id: "effect.curves",
    title: "Curves",
    category: NodeCategory::Color,
    description: "Bends the tones with a curve for all channels and for each one.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::curve("rgb", "RGB"),
        ParamSpec::curve("red", "Red"),
        ParamSpec::curve("green", "Green"),
        ParamSpec::curve("blue", "Blue"),
    ],
};

impl Simple for Curves {
    fn info(&self) -> &'static NodeInfo {
        &CURVES
    }
    fn kernel(&self) -> &'static Kernel {
        &TABLE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let master = p.curve("rgb");
        let chans = [p.curve("red"), p.curve("green"), p.curve("blue")];
        if is_identity_curve(&master) && chans.iter().all(|c| is_identity_curve(c)) {
            return None;
        }
        // the channel's own curve first, then the composite one
        Some(Setup::default().with_data(bake(|ch, x| spline(&master, spline(&chans[ch], x)))))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        table_pixel(s, cx, input.get(x, y))
    }
}

// ---- brightness and contrast ----

pub struct BrightnessContrast;

static BRIGHTNESS: NodeInfo = NodeInfo {
    id: "effect.brightness_contrast",
    title: "Brightness/Contrast",
    category: NodeCategory::Color,
    description: "Makes the picture lighter or darker and its tones further apart or closer.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::float("brightness", "Brightness", -150.0, 150.0, 0.0),
        ParamSpec::float("contrast", "Contrast", -50.0, 100.0, 0.0),
        ParamSpec::boolean("legacy", "Use legacy", false),
    ],
};

/// the tone curve brightness and contrast make. the modern version keeps black and white
/// where they are, like photoshop's; legacy shifts and stretches every value.
pub fn brightness_contrast(b: f32, c: f32, legacy: bool, x: f32) -> f32 {
    if legacy {
        let k = if c > 0.0 {
            1.0 / (1.0 - c / 100.0 * 0.99)
        } else {
            1.0 + c / 100.0
        };
        return ((x + b / 255.0 - 0.5) * k + 0.5).clamp(0.0, 1.0);
    }
    let mut v = x.clamp(0.0, 1.0).powf(2f32.powf(-b / 100.0));
    if c > 0.0 {
        v = mix(v, smoothstep(0.0, 1.0, v), c / 100.0);
    } else if c < 0.0 {
        v = mix(v, 0.5 + (v - 0.5) * 0.5, -c / 50.0);
    }
    v
}

impl Simple for BrightnessContrast {
    fn info(&self) -> &'static NodeInfo {
        &BRIGHTNESS
    }
    fn kernel(&self) -> &'static Kernel {
        &TABLE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (b, c, legacy) = (p.float("brightness"), p.float("contrast"), p.bool("legacy"));
        if b == 0.0 && c == 0.0 {
            return None;
        }
        Some(Setup::default().with_data(bake(|_, x| brightness_contrast(b, c, legacy, x))))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        table_pixel(s, cx, input.get(x, y))
    }
}

// ---- invert ----

pub struct Invert;

static INVERT: NodeInfo = NodeInfo {
    id: "effect.invert",
    title: "Invert",
    category: NodeCategory::Color,
    description: "Turns every colour into its opposite, like a negative.",
    inputs: ONE_INPUT,
    params: &[],
};

impl Simple for Invert {
    fn info(&self) -> &'static NodeInfo {
        &INVERT
    }
    fn kernel(&self) -> &'static Kernel {
        &TABLE_KERNEL
    }
    fn setup(&self, _cx: &NodeCx) -> Option<Setup> {
        // values past [0, 1] in linear documents carry on with the table's slope
        Some(Setup::default().with_data(bake(|_, x| 1.0 - x)))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        table_pixel(s, cx, input.get(x, y))
    }
}

// ---- posterize ----

pub struct Posterize;

static POSTERIZE: NodeInfo = NodeInfo {
    id: "effect.posterize",
    title: "Posterize",
    category: NodeCategory::Color,
    description: "Cuts each channel down to a few flat levels.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::int("levels", "Levels", 2, 255, 4)],
};

pub fn posterize(levels: i32, x: f32) -> f32 {
    let n = levels.max(2) as f32;
    ((x.clamp(0.0, 1.0) * n).floor().min(n - 1.0)) / (n - 1.0)
}

/// posterize in a table: the steps sit between table entries, so it gets its own kernel
const POSTERIZE_KERNEL: Kernel = point_kernel!(
    "posterize",
    r#"
fn post1(v: f32, n: f32) -> f32 {
    return min(floor(clamp(v, 0.0, 1.0) * n), n - 1.0) / (n - 1.0);
}
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let n = param(0u).x;
    return vec4<f32>(post1(c.r, n), post1(c.g, n), post1(c.b, n), c.a);
}
"#
);

impl Simple for Posterize {
    fn info(&self) -> &'static NodeInfo {
        &POSTERIZE
    }
    fn kernel(&self) -> &'static Kernel {
        &POSTERIZE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let n = cx.params.int("levels").max(2) as f32;
        Some(Setup::new(vec![[n, 0.0, 0.0, 0.0]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let n = s.p(0)[0] as i32;
        point(cx, input.get(x, y), |c| {
            [
                posterize(n, c[0]),
                posterize(n, c[1]),
                posterize(n, c[2]),
                c[3],
            ]
        })
    }
}

// ---- threshold ----

pub struct Threshold;

static THRESHOLD: NodeInfo = NodeInfo {
    id: "effect.threshold",
    title: "Threshold",
    category: NodeCategory::Color,
    description: "Turns the picture into pure black and white at a chosen brightness.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::float(
        "level",
        "Threshold level",
        1.0,
        255.0,
        128.0,
    )],
};

const THRESHOLD_KERNEL: Kernel = point_kernel!(
    "threshold",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let v = select(0.0, 1.0, clamp(luma(c.rgb), 0.0, 1.0) >= param(0u).x);
    return vec4<f32>(v, v, v, c.a);
}
"#
);

impl Simple for Threshold {
    fn info(&self) -> &'static NodeInfo {
        &THRESHOLD
    }
    fn kernel(&self) -> &'static Kernel {
        &THRESHOLD_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        Some(Setup::new(vec![[
            cx.params.float("level") / 255.0,
            0.0,
            0.0,
            0.0,
        ]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let t = s.p(0)[0];
        point(cx, input.get(x, y), |c| {
            let l = luma([c[0], c[1], c[2]]).clamp(0.0, 1.0);
            let v = if l >= t { 1.0 } else { 0.0 };
            [v, v, v, c[3]]
        })
    }
}

// ---- exposure ----

pub struct Exposure;

static EXPOSURE: NodeInfo = NodeInfo {
    id: "effect.exposure",
    title: "Exposure",
    category: NodeCategory::Color,
    description: "Changes exposure in stops, in linear light like a camera.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::float("exposure", "Exposure", -20.0, 20.0, 0.0),
        ParamSpec::float("offset", "Offset", -0.5, 0.5, 0.0),
        ParamSpec::float("gamma", "Gamma correction", 0.01, 9.99, 1.0),
    ],
};

const EXPOSURE_KERNEL: Kernel = point_kernel!(
    "exposure",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    var l = to_linear(c.rgb) * a.x + vec3<f32>(a.y);
    l = sign(l) * pow(abs(l), vec3<f32>(a.z));
    return vec4<f32>(from_linear(l), c.a);
}
"#
);

impl Simple for Exposure {
    fn info(&self) -> &'static NodeInfo {
        &EXPOSURE
    }
    fn kernel(&self) -> &'static Kernel {
        &EXPOSURE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (e, o, g) = (p.float("exposure"), p.float("offset"), p.float("gamma"));
        if e == 0.0 && o == 0.0 && g == 1.0 {
            return None;
        }
        Some(Setup::new(vec![[2f32.powf(e), o, 1.0 / g, 0.0]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let enc = cx.working.encoding;
        point(cx, input.get(x, y), |c| {
            let mut out = c;
            for k in 0..3 {
                let l = enc.decode(c[k]) * a[0] + a[1];
                out[k] = enc.encode(l.signum() * l.abs().powf(a[2]));
            }
            out
        })
    }
}

// ---- vibrance ----

pub struct Vibrance;

static VIBRANCE: NodeInfo = NodeInfo {
    id: "effect.vibrance",
    title: "Vibrance",
    category: NodeCategory::Color,
    description: "Makes dull colours richer while leaving strong ones mostly alone.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::float("vibrance", "Vibrance", -100.0, 100.0, 0.0),
        ParamSpec::float("saturation", "Saturation", -100.0, 100.0, 0.0),
    ],
};

const VIBRANCE_KERNEL: Kernel = point_kernel!(
    "vibrance",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let s = max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
    var amt = a.x;
    if (amt > 0.0) { amt = amt * (1.0 - clamp(s, 0.0, 1.0)); }
    let l = luma(c.rgb);
    var rgb = vec3<f32>(l) + (c.rgb - vec3<f32>(l)) * (1.0 + amt);
    rgb = vec3<f32>(l) + (rgb - vec3<f32>(l)) * (1.0 + a.y);
    return vec4<f32>(rgb, c.a);
}
"#
);

impl Simple for Vibrance {
    fn info(&self) -> &'static NodeInfo {
        &VIBRANCE
    }
    fn kernel(&self) -> &'static Kernel {
        &VIBRANCE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (v, s) = (p.float("vibrance") / 100.0, p.float("saturation") / 100.0);
        if v == 0.0 && s == 0.0 {
            return None;
        }
        Some(Setup::new(vec![[v, s, 0.0, 0.0]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        point(cx, input.get(x, y), |c| vibrance(c, a[0], a[1]))
    }
}

/// vibrance then saturation on a straight colour, shared with camera raw
pub fn vibrance(c: [f32; 4], vib: f32, sat: f32) -> [f32; 4] {
    let s = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let amt = if vib > 0.0 {
        vib * (1.0 - s.clamp(0.0, 1.0))
    } else {
        vib
    };
    let l = luma([c[0], c[1], c[2]]);
    let mut out = c;
    for v in out.iter_mut().take(3) {
        let a = l + (*v - l) * (1.0 + amt);
        *v = l + (a - l) * (1.0 + sat);
    }
    out
}

// ---- colour balance ----

pub struct ColorBalance;

static COLOR_BALANCE: NodeInfo = NodeInfo {
    id: "effect.color_balance",
    title: "Color Balance",
    category: NodeCategory::Color,
    description: "Pushes shadows, midtones and highlights towards a colour.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::float(
            "shadows_cyan_red",
            "Shadows: cyan to red",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "shadows_magenta_green",
            "Shadows: magenta to green",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "shadows_yellow_blue",
            "Shadows: yellow to blue",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "midtones_cyan_red",
            "Midtones: cyan to red",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "midtones_magenta_green",
            "Midtones: magenta to green",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "midtones_yellow_blue",
            "Midtones: yellow to blue",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "highlights_cyan_red",
            "Highlights: cyan to red",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "highlights_magenta_green",
            "Highlights: magenta to green",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::float(
            "highlights_yellow_blue",
            "Highlights: yellow to blue",
            -100.0,
            100.0,
            0.0,
        ),
        ParamSpec::boolean("preserve_luminosity", "Preserve luminosity", true),
    ],
};

const COLOR_BALANCE_KERNEL: Kernel = point_kernel!(
    "color_balance",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    let l = clamp((mx + mn) * 0.5, 0.0, 1.0);
    let a = 0.25;
    let b = 0.333;
    let sh = clamp((l - b) / -a + 0.5, 0.0, 1.0) * 0.7;
    let md = clamp((l - b) / a + 0.5, 0.0, 1.0) * clamp((l + b - 1.0) / -a + 0.5, 0.0, 1.0) * 0.7;
    let hi = clamp((l + b - 1.0) / a + 0.5, 0.0, 1.0) * 0.7;
    var rgb = c.rgb + sh * param(0u).xyz + md * param(1u).xyz + hi * param(2u).xyz;
    if (param(0u).w > 0.5) { rgb = set_lum(rgb, luma(c.rgb), clamps()); }
    return vec4<f32>(rgb, c.a);
}
"#
);

impl Simple for ColorBalance {
    fn info(&self) -> &'static NodeInfo {
        &COLOR_BALANCE
    }
    fn kernel(&self) -> &'static Kernel {
        &COLOR_BALANCE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let row = |t: &str| {
            [
                p.float(&format!("{t}_cyan_red")) / 100.0,
                p.float(&format!("{t}_magenta_green")) / 100.0,
                p.float(&format!("{t}_yellow_blue")) / 100.0,
            ]
        };
        let (s, m, h) = (row("shadows"), row("midtones"), row("highlights"));
        if s.iter().chain(&m).chain(&h).all(|v| *v == 0.0) {
            return None;
        }
        let keep = f32::from(u8::from(p.bool("preserve_luminosity")));
        Some(Setup::new(vec![
            [s[0], s[1], s[2], keep],
            [m[0], m[1], m[2], 0.0],
            [h[0], h[1], h[2], 0.0],
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let clamp = cx.clamp();
        point(cx, input.get(x, y), |c| {
            let mx = c[0].max(c[1]).max(c[2]);
            let mn = c[0].min(c[1]).min(c[2]);
            let l = ((mx + mn) * 0.5).clamp(0.0, 1.0);
            let (a, b) = (0.25, 0.333);
            let sh = ((l - b) / -a + 0.5).clamp(0.0, 1.0) * 0.7;
            let md = ((l - b) / a + 0.5).clamp(0.0, 1.0)
                * ((l + b - 1.0) / -a + 0.5).clamp(0.0, 1.0)
                * 0.7;
            let hi = ((l + b - 1.0) / a + 0.5).clamp(0.0, 1.0) * 0.7;
            let mut rgb = [0.0; 3];
            for (k, v) in rgb.iter_mut().enumerate() {
                *v = c[k] + sh * s.p(0)[k] + md * s.p(1)[k] + hi * s.p(2)[k];
            }
            if s.p(0)[3] > 0.5 {
                rgb = set_lum(rgb, luma([c[0], c[1], c[2]]), clamp);
            }
            [rgb[0], rgb[1], rgb[2], c[3]]
        })
    }
}

// ---- black and white ----

pub struct BlackWhite;

static BLACK_WHITE: NodeInfo = NodeInfo {
    id: "effect.black_white",
    title: "Black & White",
    category: NodeCategory::Color,
    description: "Turns colour into grey, with a slider for how light each colour becomes.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("reds", "Reds", -200.0, 300.0, 40.0),
        ParamSpec::percent("yellows", "Yellows", -200.0, 300.0, 60.0),
        ParamSpec::percent("greens", "Greens", -200.0, 300.0, 40.0),
        ParamSpec::percent("cyans", "Cyans", -200.0, 300.0, 60.0),
        ParamSpec::percent("blues", "Blues", -200.0, 300.0, 20.0),
        ParamSpec::percent("magentas", "Magentas", -200.0, 300.0, 80.0),
        ParamSpec::boolean("tint", "Tint", false),
        ParamSpec::degrees("tint_hue", "Tint hue", 0.0, 360.0, 42.0),
        ParamSpec::percent("tint_saturation", "Tint saturation", 0.0, 100.0, 20.0),
    ],
};

/// photoshop's black and white: the smallest channel, plus the step to the middle one
/// weighted by the secondary colour, plus the step to the largest weighted by the primary
const BLACK_WHITE_KERNEL: Kernel = point_kernel!(
    "black_white",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let w0 = param(0u);
    let w1 = param(1u);
    let r = c.r; let g = c.g; let b = c.b;
    var gray: f32;
    if (r >= g && g >= b) { gray = b + (g - b) * w0.y + (r - g) * w0.x; }
    else if (r >= b && b >= g) { gray = g + (b - g) * w1.y + (r - b) * w0.x; }
    else if (g >= r && r >= b) { gray = b + (r - b) * w0.y + (g - r) * w0.z; }
    else if (g >= b && b >= r) { gray = r + (b - r) * w0.w + (g - b) * w0.z; }
    else if (b >= g && g >= r) { gray = r + (g - r) * w0.w + (b - g) * w1.x; }
    else { gray = g + (r - g) * w1.y + (b - r) * w1.x; }
    var rgb = vec3<f32>(gray);
    if (w1.z > 0.5) {
        let tinted = set_lum(param(2u).xyz, gray, clamps());
        rgb = mix(rgb, tinted, w1.w);
    }
    return vec4<f32>(rgb, c.a);
}
"#
);

pub fn black_white_gray(c: [f32; 3], w: [f32; 6]) -> f32 {
    let [wr, wy, wg, wc, wb, wm] = w;
    let [r, g, b] = c;
    if r >= g && g >= b {
        b + (g - b) * wy + (r - g) * wr
    } else if r >= b && b >= g {
        g + (b - g) * wm + (r - b) * wr
    } else if g >= r && r >= b {
        b + (r - b) * wy + (g - r) * wg
    } else if g >= b && b >= r {
        r + (b - r) * wc + (g - b) * wg
    } else if b >= g && g >= r {
        r + (g - r) * wc + (b - g) * wb
    } else {
        g + (r - g) * wm + (b - r) * wb
    }
}

impl Simple for BlackWhite {
    fn info(&self) -> &'static NodeInfo {
        &BLACK_WHITE
    }
    fn kernel(&self) -> &'static Kernel {
        &BLACK_WHITE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let w = |id: &str| p.float(id) / 100.0;
        let tint = hsl_to_rgb([p.float("tint_hue") / 360.0, 1.0, 0.5]);
        Some(Setup::new(vec![
            [w("reds"), w("yellows"), w("greens"), w("cyans")],
            [
                w("blues"),
                w("magentas"),
                f32::from(u8::from(p.bool("tint"))),
                w("tint_saturation"),
            ],
            [tint[0], tint[1], tint[2], 0.0],
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (w0, w1, t) = (s.p(0), s.p(1), s.p(2));
        let clamp = cx.clamp();
        point(cx, input.get(x, y), |c| {
            let gray = black_white_gray(
                [c[0], c[1], c[2]],
                [w0[0], w0[1], w0[2], w0[3], w1[0], w1[1]],
            );
            let mut rgb = [gray; 3];
            if w1[2] > 0.5 {
                let tinted = set_lum([t[0], t[1], t[2]], gray, clamp);
                rgb = mix3(rgb, tinted, w1[3]);
            }
            [rgb[0], rgb[1], rgb[2], c[3]]
        })
    }
}

// ---- photo filter ----

pub struct PhotoFilter;

static PHOTO_FILTER: NodeInfo = NodeInfo {
    id: "effect.photo_filter",
    title: "Photo Filter",
    category: NodeCategory::Color,
    description: "Tints the picture like a coloured filter in front of the lens.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::color("color", "Filter colour", [0.925, 0.541, 0.0, 1.0]),
        ParamSpec::percent("density", "Density", 0.0, 100.0, 25.0),
        ParamSpec::boolean("preserve_luminosity", "Preserve luminosity", true),
    ],
};

const PHOTO_FILTER_KERNEL: Kernel = point_kernel!(
    "photo_filter",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let f = param(0u);
    var rgb = mix(c.rgb, c.rgb * f.rgb, f.w);
    if (param(1u).x > 0.5) { rgb = set_lum(rgb, luma(c.rgb), clamps()); }
    return vec4<f32>(rgb, c.a);
}
"#
);

impl Simple for PhotoFilter {
    fn info(&self) -> &'static NodeInfo {
        &PHOTO_FILTER
    }
    fn kernel(&self) -> &'static Kernel {
        &PHOTO_FILTER_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let d = p.float("density") / 100.0;
        if d == 0.0 {
            return None;
        }
        let f = working_color(cx, p.color("color"));
        let keep = f32::from(u8::from(p.bool("preserve_luminosity")));
        Some(Setup::new(vec![
            [f[0], f[1], f[2], d],
            [keep, 0.0, 0.0, 0.0],
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (f, keep) = (s.p(0), s.p(1)[0] > 0.5);
        let clamp = cx.clamp();
        point(cx, input.get(x, y), |c| {
            let mut rgb = [0.0; 3];
            for (k, v) in rgb.iter_mut().enumerate() {
                *v = mix(c[k], c[k] * f[k], f[3]);
            }
            if keep {
                rgb = set_lum(rgb, luma([c[0], c[1], c[2]]), clamp);
            }
            [rgb[0], rgb[1], rgb[2], c[3]]
        })
    }
}

// ---- channel mixer ----

pub struct ChannelMixer;

static CHANNEL_MIXER: NodeInfo = NodeInfo {
    id: "effect.channel_mixer",
    title: "Channel Mixer",
    category: NodeCategory::Color,
    description: "Builds each channel from a mix of the red, green and blue channels.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("red_red", "Red from red", -200.0, 200.0, 100.0),
        ParamSpec::percent("red_green", "Red from green", -200.0, 200.0, 0.0),
        ParamSpec::percent("red_blue", "Red from blue", -200.0, 200.0, 0.0),
        ParamSpec::percent("red_constant", "Red constant", -200.0, 200.0, 0.0),
        ParamSpec::percent("green_red", "Green from red", -200.0, 200.0, 0.0),
        ParamSpec::percent("green_green", "Green from green", -200.0, 200.0, 100.0),
        ParamSpec::percent("green_blue", "Green from blue", -200.0, 200.0, 0.0),
        ParamSpec::percent("green_constant", "Green constant", -200.0, 200.0, 0.0),
        ParamSpec::percent("blue_red", "Blue from red", -200.0, 200.0, 0.0),
        ParamSpec::percent("blue_green", "Blue from green", -200.0, 200.0, 0.0),
        ParamSpec::percent("blue_blue", "Blue from blue", -200.0, 200.0, 100.0),
        ParamSpec::percent("blue_constant", "Blue constant", -200.0, 200.0, 0.0),
        ParamSpec::boolean("monochrome", "Monochrome", false),
    ],
};

const CHANNEL_MIXER_KERNEL: Kernel = point_kernel!(
    "channel_mixer",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let r = param(0u); let g = param(1u); let b = param(2u);
    let v = vec4<f32>(c.rgb, 1.0);
    return vec4<f32>(dot(r, v), dot(g, v), dot(b, v), c.a);
}
"#
);

impl Simple for ChannelMixer {
    fn info(&self) -> &'static NodeInfo {
        &CHANNEL_MIXER
    }
    fn kernel(&self) -> &'static Kernel {
        &CHANNEL_MIXER_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let row = |out: &str| {
            ["red", "green", "blue", "constant"]
                .map(|from| p.float(&format!("{out}_{from}")) / 100.0)
        };
        let r = row("red");
        let rows = if p.bool("monochrome") {
            vec![r, r, r]
        } else {
            vec![r, row("green"), row("blue")]
        };
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        if rows == identity {
            return None;
        }
        Some(Setup::new(rows))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        point(cx, input.get(x, y), |c| {
            let m = |r: [f32; 4]| r[0] * c[0] + r[1] * c[1] + r[2] * c[2] + r[3];
            [m(s.p(0)), m(s.p(1)), m(s.p(2)), c[3]]
        })
    }
}

// ---- gradient map ----

pub struct GradientMap;

const BLACK_TO_WHITE: &[GradientStop] = &[
    GradientStop {
        offset: 0.0,
        color: [0.0, 0.0, 0.0, 1.0],
    },
    GradientStop {
        offset: 1.0,
        color: [1.0, 1.0, 1.0, 1.0],
    },
];

static GRADIENT_MAP: NodeInfo = NodeInfo {
    id: "effect.gradient_map",
    title: "Gradient Map",
    category: NodeCategory::Color,
    description: "Paints each brightness with a colour from a gradient.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::gradient("gradient", "Gradient", BLACK_TO_WHITE),
        ParamSpec::boolean("reverse", "Reverse", false),
    ],
};

const GRADIENT_ENTRIES: usize = 256;

const GRADIENT_MAP_KERNEL: Kernel = point_kernel!(
    "gradient_map",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let n = 256u;
    let t = clamp(luma(c.rgb), 0.0, 1.0);
    let g = vec4<f32>(table_at(0u, n, t), table_at(n, n, t), table_at(2u * n, n, t), table_at(3u * n, n, t));
    return vec4<f32>(g.rgb, c.a * g.a);
}
"#
);

/// a gradient's colour at t, straight, from sorted stops
pub fn gradient_at(stops: &[GradientStop], t: f32) -> [f32; 4] {
    let Some(first) = stops.first() else {
        return [t, t, t, 1.0];
    };
    if t <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t <= b.offset {
            let k = ((t - a.offset) / (b.offset - a.offset).max(1e-6)).clamp(0.0, 1.0);
            let mut c = [0.0; 4];
            for (i, v) in c.iter_mut().enumerate() {
                *v = mix(a.color[i], b.color[i], k);
            }
            return c;
        }
    }
    stops.last().map_or(first.color, |s| s.color)
}

impl Simple for GradientMap {
    fn info(&self) -> &'static NodeInfo {
        &GRADIENT_MAP
    }
    fn kernel(&self) -> &'static Kernel {
        &GRADIENT_MAP_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let stops = cx.params.gradient("gradient");
        let reverse = cx.params.bool("reverse");
        let mut data = vec![0.0; 4 * GRADIENT_ENTRIES];
        for i in 0..GRADIENT_ENTRIES {
            let mut t = i as f32 / (GRADIENT_ENTRIES - 1) as f32;
            if reverse {
                t = 1.0 - t;
            }
            let c = gradient_at(&stops, t);
            let w = working_color(cx, c);
            for k in 0..3 {
                data[k * GRADIENT_ENTRIES + i] = w[k];
            }
            data[3 * GRADIENT_ENTRIES + i] = c[3];
        }
        Some(Setup::default().with_data(data))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let n = GRADIENT_ENTRIES;
        point(cx, input.get(x, y), |c| {
            let t = luma([c[0], c[1], c[2]]).clamp(0.0, 1.0);
            let g = [0, 1, 2, 3].map(|k| table_at(&s.data, k * n, n, t));
            [g[0], g[1], g[2], c[3] * g[3]]
        })
    }
}

// ---- selective colour ----

pub struct SelectiveColor;

pub const SELECTIVE_RANGES: [&str; 9] = [
    "reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks",
];

pub const SELECTIVE_METHODS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "relative",
        label: "Relative",
    },
    ChoiceOption {
        id: "absolute",
        label: "Absolute",
    },
];

macro_rules! ink {
    ($id:literal, $label:literal) => {
        ParamSpec::percent($id, $label, -100.0, 100.0, 0.0)
    };
}

static SELECTIVE_COLOR: NodeInfo = NodeInfo {
    id: "effect.selective_color",
    title: "Selective Color",
    category: NodeCategory::Color,
    description: "Adds or removes cyan, magenta, yellow and black ink in one range of colours.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("method", "Method", SELECTIVE_METHODS, "relative"),
        ink!("reds_cyan", "Reds: cyan"),
        ink!("reds_magenta", "Reds: magenta"),
        ink!("reds_yellow", "Reds: yellow"),
        ink!("reds_black", "Reds: black"),
        ink!("yellows_cyan", "Yellows: cyan"),
        ink!("yellows_magenta", "Yellows: magenta"),
        ink!("yellows_yellow", "Yellows: yellow"),
        ink!("yellows_black", "Yellows: black"),
        ink!("greens_cyan", "Greens: cyan"),
        ink!("greens_magenta", "Greens: magenta"),
        ink!("greens_yellow", "Greens: yellow"),
        ink!("greens_black", "Greens: black"),
        ink!("cyans_cyan", "Cyans: cyan"),
        ink!("cyans_magenta", "Cyans: magenta"),
        ink!("cyans_yellow", "Cyans: yellow"),
        ink!("cyans_black", "Cyans: black"),
        ink!("blues_cyan", "Blues: cyan"),
        ink!("blues_magenta", "Blues: magenta"),
        ink!("blues_yellow", "Blues: yellow"),
        ink!("blues_black", "Blues: black"),
        ink!("magentas_cyan", "Magentas: cyan"),
        ink!("magentas_magenta", "Magentas: magenta"),
        ink!("magentas_yellow", "Magentas: yellow"),
        ink!("magentas_black", "Magentas: black"),
        ink!("whites_cyan", "Whites: cyan"),
        ink!("whites_magenta", "Whites: magenta"),
        ink!("whites_yellow", "Whites: yellow"),
        ink!("whites_black", "Whites: black"),
        ink!("neutrals_cyan", "Neutrals: cyan"),
        ink!("neutrals_magenta", "Neutrals: magenta"),
        ink!("neutrals_yellow", "Neutrals: yellow"),
        ink!("neutrals_black", "Neutrals: black"),
        ink!("blacks_cyan", "Blacks: cyan"),
        ink!("blacks_magenta", "Blacks: magenta"),
        ink!("blacks_yellow", "Blacks: yellow"),
        ink!("blacks_black", "Blacks: black"),
    ],
};

/// how much each range claims a colour. primaries own the step from the largest channel to
/// the middle one, secondaries the step from the middle one to the smallest.
const SELECTIVE_KERNEL: Kernel = point_kernel!(
    "selective_color",
    r#"
fn sel_weight(i: u32, c: vec3<f32>) -> f32 {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    let md = c.r + c.g + c.b - mx - mn;
    switch i {
        case 0u: { return select(0.0, mx - md, c.r == mx && mx > mn); }
        case 1u: { return select(0.0, md - mn, c.b == mn && mx > mn); }
        case 2u: { return select(0.0, mx - md, c.g == mx && c.r != mx && mx > mn); }
        case 3u: { return select(0.0, md - mn, c.r == mn && c.b != mn && mx > mn); }
        case 4u: { return select(0.0, mx - md, c.b == mx && c.r != mx && c.g != mx && mx > mn); }
        case 5u: { return select(0.0, md - mn, c.g == mn && c.b != mn && c.r != mn && mx > mn); }
        case 6u: { return select(0.0, (mn - 0.5) * 2.0, mn > 0.5); }
        case 7u: { return max(1.0 - (abs(mx - 0.5) + abs(mn - 0.5)), 0.0); }
        default: { return select(0.0, (0.5 - mx) * 2.0, mx < 0.5); }
    }
}

fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let relative = param(9u).x < 0.5;
    let v = clamp01(c.rgb);
    var out = v;
    for (var i = 0u; i < 9u; i = i + 1u) {
        let w = sel_weight(i, v);
        if (w <= 0.0) { continue; }
        let a = param(i);
        for (var k = 0u; k < 3u; k = k + 1u) {
            let ink = 1.0 - v[k];
            var next: f32;
            if (relative) { next = ink * (1.0 + a[k]); } else { next = ink + a[k]; }
            if (a.w >= 0.0) { next = next + a.w * (1.0 - next); } else { next = next * (1.0 + a.w); }
            out[k] = out[k] + w * ((1.0 - clamp(next, 0.0, 1.0)) - v[k]);
        }
    }
    return vec4<f32>(out, c.a);
}
"#
);

fn selective_weight(i: usize, c: [f32; 3]) -> f32 {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let md = c[0] + c[1] + c[2] - mx - mn;
    let some = mx > mn;
    let pick = |cond: bool, v: f32| if cond { v } else { 0.0 };
    match i {
        0 => pick(c[0] == mx && some, mx - md),
        1 => pick(c[2] == mn && some, md - mn),
        2 => pick(c[1] == mx && c[0] != mx && some, mx - md),
        3 => pick(c[0] == mn && c[2] != mn && some, md - mn),
        4 => pick(c[2] == mx && c[0] != mx && c[1] != mx && some, mx - md),
        5 => pick(c[1] == mn && c[2] != mn && c[0] != mn && some, md - mn),
        6 => pick(mn > 0.5, (mn - 0.5) * 2.0),
        7 => (1.0 - ((mx - 0.5).abs() + (mn - 0.5).abs())).max(0.0),
        _ => pick(mx < 0.5, (0.5 - mx) * 2.0),
    }
}

impl Simple for SelectiveColor {
    fn info(&self) -> &'static NodeInfo {
        &SELECTIVE_COLOR
    }
    fn kernel(&self) -> &'static Kernel {
        &SELECTIVE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let mut params: Vec<[f32; 4]> = SELECTIVE_RANGES
            .iter()
            .map(|r| {
                ["cyan", "magenta", "yellow", "black"].map(|i| p.float(&format!("{r}_{i}")) / 100.0)
            })
            .collect();
        if params.iter().flatten().all(|v| *v == 0.0) {
            return None;
        }
        params.push([p.choice_index("method") as f32, 0.0, 0.0, 0.0]);
        Some(Setup::new(params))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let relative = s.p(9)[0] < 0.5;
        point(cx, input.get(x, y), |c| {
            let v = clamp01([c[0], c[1], c[2]]);
            let mut out = v;
            for i in 0..9 {
                let w = selective_weight(i, v);
                if w <= 0.0 {
                    continue;
                }
                let a = s.p(i);
                for k in 0..3 {
                    let ink = 1.0 - v[k];
                    let mut next = if relative {
                        ink * (1.0 + a[k])
                    } else {
                        ink + a[k]
                    };
                    next = if a[3] >= 0.0 {
                        next + a[3] * (1.0 - next)
                    } else {
                        next * (1.0 + a[3])
                    };
                    out[k] += w * ((1.0 - next.clamp(0.0, 1.0)) - v[k]);
                }
            }
            [out[0], out[1], out[2], c[3]]
        })
    }
}

// ---- match colour ----

pub struct MatchColor;

static MATCH_COLOR: NodeInfo = NodeInfo {
    id: "effect.match_color",
    title: "Match Color",
    category: NodeCategory::Color,
    description: "Gives the picture the overall colour and contrast of another one.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("luminance", "Luminance", 1.0, 200.0, 100.0),
        ParamSpec::percent("intensity", "Color intensity", 1.0, 200.0, 100.0),
        ParamSpec::percent("fade", "Fade", 0.0, 100.0, 0.0),
        ParamSpec::boolean("neutralize", "Neutralize", false),
        // worked out from the pictures when the adjustment is made
        ParamSpec::color("source_mean", "Source average", [0.5, 0.0, 0.0, 0.0]).hide(),
        ParamSpec::color("source_spread", "Source spread", [0.25, 0.1, 0.1, 0.0]).hide(),
        ParamSpec::color("target_mean", "Target average", [0.5, 0.0, 0.0, 0.0]).hide(),
        ParamSpec::color("target_spread", "Target spread", [0.25, 0.1, 0.1, 0.0]).hide(),
    ],
};

/// a colour as luma and two colour differences, the space match colour works in
pub fn to_opponent(c: [f32; 3]) -> [f32; 3] {
    let y = luma(c);
    [y, c[2] - y, c[0] - y]
}

pub fn from_opponent(o: [f32; 3]) -> [f32; 3] {
    let (y, u, v) = (o[0], o[1], o[2]);
    let r = v + y;
    let b = u + y;
    let g = (y - 0.3 * r - 0.11 * b) / 0.59;
    [r, g, b]
}

const MATCH_COLOR_KERNEL: Kernel = point_kernel!(
    "match_color",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let sm = param(0u).xyz; let ss = param(1u).xyz;
    let tm = param(2u).xyz; let ts = param(3u).xyz;
    let k = param(4u);
    let y = luma(c.rgb);
    let o = vec3<f32>(y, c.b - y, c.r - y);
    var n = (o - tm) / max(ts, vec3<f32>(1e-4)) * ss + sm;
    n.x = n.x * k.x;
    n = vec3<f32>(n.x, n.y * k.y, n.z * k.y);
    let r = n.z + n.x;
    let b = n.y + n.x;
    let g = (n.x - 0.3 * r - 0.11 * b) / 0.59;
    return vec4<f32>(mix(vec3<f32>(r, g, b), c.rgb, k.z), c.a);
}
"#
);

impl Simple for MatchColor {
    fn info(&self) -> &'static NodeInfo {
        &MATCH_COLOR
    }
    fn kernel(&self) -> &'static Kernel {
        &MATCH_COLOR_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let mut sm = p.color("source_mean");
        if p.bool("neutralize") {
            sm[1] = 0.0;
            sm[2] = 0.0;
        }
        Some(Setup::new(vec![
            sm,
            p.color("source_spread"),
            p.color("target_mean"),
            p.color("target_spread"),
            [
                p.float("luminance") / 100.0,
                p.float("intensity") / 100.0,
                p.float("fade") / 100.0,
                0.0,
            ],
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (sm, ss, tm, ts, k) = (s.p(0), s.p(1), s.p(2), s.p(3), s.p(4));
        point(cx, input.get(x, y), |c| {
            let o = to_opponent([c[0], c[1], c[2]]);
            let mut n = [0.0; 3];
            for i in 0..3 {
                n[i] = (o[i] - tm[i]) / ts[i].max(1e-4) * ss[i] + sm[i];
            }
            n[0] *= k[0];
            n[1] *= k[1];
            n[2] *= k[1];
            let rgb = from_opponent(n);
            let out = mix3(rgb, [c[0], c[1], c[2]], k[2]);
            [out[0], out[1], out[2], c[3]]
        })
    }
}

/// the average and spread of colours in the space match colour uses, for its parameters
pub fn opponent_stats(pixels: impl Iterator<Item = [f32; 3]>) -> ([f32; 4], [f32; 4]) {
    let (mut n, mut sum, mut sq) = (0.0f64, [0.0f64; 3], [0.0f64; 3]);
    for c in pixels {
        let o = to_opponent(c);
        for k in 0..3 {
            sum[k] += f64::from(o[k]);
            sq[k] += f64::from(o[k]) * f64::from(o[k]);
        }
        n += 1.0;
    }
    if n == 0.0 {
        return ([0.5, 0.0, 0.0, 0.0], [0.25, 0.1, 0.1, 0.0]);
    }
    let mut mean = [0.0f32; 4];
    let mut spread = [0.0f32; 4];
    for k in 0..3 {
        let m = sum[k] / n;
        mean[k] = m as f32;
        spread[k] = ((sq[k] / n - m * m).max(0.0).sqrt() as f32).max(1e-3);
    }
    (mean, spread)
}

// ---- replace colour ----

pub struct ReplaceColor;

static REPLACE_COLOR: NodeInfo = NodeInfo {
    id: "effect.replace_color",
    title: "Replace Color",
    category: NodeCategory::Color,
    description: "Picks out one colour and changes its hue, saturation and lightness.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::color("color", "Colour", [0.8, 0.1, 0.1, 1.0]),
        ParamSpec::float("fuzziness", "Fuzziness", 0.0, 200.0, 40.0),
        ParamSpec::degrees("hue", "Hue", -180.0, 180.0, 0.0),
        ParamSpec::float("saturation", "Saturation", -100.0, 100.0, 0.0),
        ParamSpec::float("lightness", "Lightness", -100.0, 100.0, 0.0),
    ],
};

/// selects like colour range: fully within half the fuzziness, nothing past it
const REPLACE_COLOR_KERNEL: Kernel = point_kernel!(
    "replace_color",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let t = param(0u);
    let a = param(1u);
    let d = abs(clamp01(c.rgb) - t.rgb);
    let dist = max(max(d.r, d.g), d.b);
    let w = 1.0 - smoothstep(t.w * 0.5, max(t.w, 1e-4), dist);
    if (w <= 0.0) { return c; }
    var hsl = rgb_to_hsl(clamp01(c.rgb));
    hsl.x = fract(hsl.x + a.x);
    hsl.y = clamp(hsl.y * (1.0 + a.y), 0.0, 1.0);
    if (a.z < 0.0) { hsl.z = hsl.z * (1.0 + a.z); } else { hsl.z = hsl.z + a.z * (1.0 - hsl.z); }
    return vec4<f32>(mix(c.rgb, hsl_to_rgb(hsl), w), c.a);
}
"#
);

impl Simple for ReplaceColor {
    fn info(&self) -> &'static NodeInfo {
        &REPLACE_COLOR
    }
    fn kernel(&self) -> &'static Kernel {
        &REPLACE_COLOR_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let a = [
            p.float("hue") / 360.0,
            p.float("saturation") / 100.0,
            p.float("lightness") / 100.0,
            0.0,
        ];
        if a == [0.0; 4] {
            return None;
        }
        let t = working_color(cx, p.color("color"));
        Some(Setup::new(vec![
            [t[0], t[1], t[2], p.float("fuzziness") / 255.0],
            a,
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (t, a) = (s.p(0), s.p(1));
        point(cx, input.get(x, y), |c| {
            let v = clamp01([c[0], c[1], c[2]]);
            let dist = (0..3).map(|k| (v[k] - t[k]).abs()).fold(0.0, f32::max);
            let w = 1.0 - smoothstep(t[3] * 0.5, t[3].max(1e-4), dist);
            if w <= 0.0 {
                return c;
            }
            let mut hsl = rgb_to_hsl(v);
            let h = hsl[0] + a[0];
            hsl[0] = h - h.floor();
            hsl[1] = (hsl[1] * (1.0 + a[1])).clamp(0.0, 1.0);
            hsl[2] = if a[2] < 0.0 {
                hsl[2] * (1.0 + a[2])
            } else {
                hsl[2] + a[2] * (1.0 - hsl[2])
            };
            let out = mix3([c[0], c[1], c[2]], hsl_to_rgb(hsl), w);
            [out[0], out[1], out[2], c[3]]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splines_pass_through_their_points() {
        let pts = [[0.0, 0.0], [0.25, 0.4], [0.75, 0.6], [1.0, 1.0]];
        for p in pts {
            assert!((spline(&pts, p[0]) - p[1]).abs() < 1e-4, "{p:?}");
        }
        assert!((spline(&[[0.0, 0.0], [1.0, 1.0]], 0.3) - 0.3).abs() < 1e-6);
        // an s curve darkens shadows and lifts highlights
        let s = [[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]];
        assert!(spline(&s, 0.2) < 0.2 && spline(&s, 0.8) > 0.8);
    }

    #[test]
    fn black_and_white_matches_photoshop_defaults() {
        let w = [0.4, 0.6, 0.4, 0.6, 0.2, 0.8];
        assert!((black_white_gray([1.0, 0.0, 0.0], w) - 0.4).abs() < 1e-6);
        assert!((black_white_gray([1.0, 1.0, 0.0], w) - 0.6).abs() < 1e-6);
        assert!((black_white_gray([0.0, 0.0, 1.0], w) - 0.2).abs() < 1e-6);
        assert!((black_white_gray([0.3, 0.3, 0.3], w) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn posterize_and_brightness_keep_the_ends() {
        assert_eq!(posterize(2, 0.2), 0.0);
        assert_eq!(posterize(2, 0.8), 1.0);
        assert_eq!(posterize(4, 1.0), 1.0);
        assert!((brightness_contrast(50.0, 0.0, false, 1.0) - 1.0).abs() < 1e-6);
        assert!(brightness_contrast(50.0, 0.0, false, 0.5) > 0.5);
        assert!(brightness_contrast(-50.0, 0.0, false, 0.5) < 0.5);
        assert!(brightness_contrast(0.0, 80.0, false, 0.2) < 0.2);
    }

    #[test]
    fn opponent_colours_round_trip() {
        for c in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.3]] {
            let back = from_opponent(to_opponent(c));
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-5);
            }
        }
    }
}
