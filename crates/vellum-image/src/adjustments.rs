//! adjustment layers: a colour node from the render engine applied to everything below,
//! kept as settings so it can be changed or removed at any time.
//!
//! an adjustment is the id of a `vellum-gpu` node and its parameter values. the window
//! builds its controls from the node's parameter list, so adding an adjustment to the
//! engine is enough for it to show up here. presets are named parameter sets; the auto
//! buttons and equalize work out parameters from the pixels below the layer.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::color::{ColorStep, Lut3d};
use vellum_gpu::graph::Node;
use vellum_gpu::nodes::adjust::opponent_stats;
use vellum_gpu::params::GradientStop;
use vellum_gpu::{ColorTransform, IRect, NodeRegistry, ParamValue, Transfer, WorkingSpace};

use crate::document::{ImageDocument, Layer, LayerId};
use crate::selection::ColorBuf;

/// a lookup table loaded from a `.cube` file, kept as text so saving gives it back
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LutTable {
    pub name: String,
    pub cube: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Adjustment {
    /// the render node that does the work, eg `effect.curves`
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
    /// the table for colour lookup
    #[serde(default)]
    pub lut: Option<LutTable>,
}

impl Adjustment {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            params: BTreeMap::new(),
            lut: None,
        }
    }

    pub fn with(mut self, id: &str, value: impl Into<ParamValue>) -> Self {
        self.params.insert(id.to_owned(), value.into());
        self
    }
}

/// what the adjustments panel offers, in photoshop's order: node id and name
pub const KINDS: &[(&str, &str)] = &[
    ("effect.brightness_contrast", "Brightness/Contrast"),
    ("effect.levels", "Levels"),
    ("effect.curves", "Curves"),
    ("effect.exposure", "Exposure"),
    ("effect.vibrance", "Vibrance"),
    ("effect.hue_saturation", "Hue/Saturation"),
    ("effect.color_balance", "Color Balance"),
    ("effect.black_white", "Black & White"),
    ("effect.photo_filter", "Photo Filter"),
    ("effect.channel_mixer", "Channel Mixer"),
    ("color.lut", "Color Lookup"),
    ("effect.invert", "Invert"),
    ("effect.posterize", "Posterize"),
    ("effect.threshold", "Threshold"),
    ("effect.gradient_map", "Gradient Map"),
    ("effect.selective_color", "Selective Color"),
    ("effect.shadows_highlights", "Shadows/Highlights"),
    ("effect.hdr_toning", "HDR Toning"),
    ("effect.match_color", "Match Color"),
    ("effect.replace_color", "Replace Color"),
    ("effect.camera_raw", "Camera Raw"),
];

pub fn title_of(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|k| k.0 == kind)
        .map_or("Adjustment", |k| k.1)
}

/// a named set of parameters for one adjustment
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preset {
    pub kind: String,
    pub name: String,
    pub params: BTreeMap<String, ParamValue>,
}

fn preset(kind: &str, name: &str, values: &[(&str, ParamValue)]) -> Preset {
    Preset {
        kind: kind.into(),
        name: name.into(),
        params: values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    }
}

fn curve(points: &[[f32; 2]]) -> ParamValue {
    ParamValue::Curve(points.to_vec())
}

fn stops(colors: &[[f32; 4]]) -> ParamValue {
    let n = colors.len().max(2) - 1;
    ParamValue::Gradient(
        colors
            .iter()
            .enumerate()
            .map(|(i, c)| GradientStop {
                offset: i as f32 / n as f32,
                color: *c,
            })
            .collect(),
    )
}

/// the built in presets, like photoshop's preset menus
pub fn presets() -> Vec<Preset> {
    let f = ParamValue::Float;
    let b = ParamValue::Bool;
    vec![
        preset(
            "effect.curves",
            "Medium contrast",
            &[(
                "rgb",
                curve(&[[0.0, 0.0], [0.25, 0.2], [0.75, 0.8], [1.0, 1.0]]),
            )],
        ),
        preset(
            "effect.curves",
            "Strong contrast",
            &[(
                "rgb",
                curve(&[[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]]),
            )],
        ),
        preset(
            "effect.curves",
            "Lighter",
            &[("rgb", curve(&[[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]]))],
        ),
        preset(
            "effect.curves",
            "Darker",
            &[("rgb", curve(&[[0.0, 0.0], [0.5, 0.4], [1.0, 1.0]]))],
        ),
        preset(
            "effect.curves",
            "Negative",
            &[("rgb", curve(&[[0.0, 1.0], [1.0, 0.0]]))],
        ),
        preset(
            "effect.curves",
            "Cross process",
            &[
                (
                    "red",
                    curve(&[[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]]),
                ),
                (
                    "green",
                    curve(&[[0.0, 0.0], [0.25, 0.2], [0.75, 0.82], [1.0, 1.0]]),
                ),
                ("blue", curve(&[[0.0, 0.15], [1.0, 0.85]])),
            ],
        ),
        preset(
            "effect.curves",
            "Faded film",
            &[("rgb", curve(&[[0.0, 0.1], [0.5, 0.5], [1.0, 0.92]]))],
        ),
        preset("effect.levels", "Darker", &[("gamma", f(0.7))]),
        preset("effect.levels", "Lighter", &[("gamma", f(1.4))]),
        preset(
            "effect.levels",
            "Increase contrast",
            &[("input_black", f(20.0)), ("input_white", f(235.0))],
        ),
        preset("effect.levels", "Midtones lighter", &[("gamma", f(1.2))]),
        preset("effect.exposure", "Minus 1.0", &[("exposure", f(-1.0))]),
        preset("effect.exposure", "Plus 1.0", &[("exposure", f(1.0))]),
        preset("effect.exposure", "Plus 2.0", &[("exposure", f(2.0))]),
        preset(
            "effect.hue_saturation",
            "Sepia",
            &[
                ("colorize", b(true)),
                ("hue", f(35.0)),
                ("saturation", f(25.0)),
            ],
        ),
        preset(
            "effect.hue_saturation",
            "Old style",
            &[("saturation", f(-40.0)), ("lightness", f(5.0))],
        ),
        preset(
            "effect.hue_saturation",
            "Strong saturation",
            &[("saturation", f(40.0))],
        ),
        preset(
            "effect.black_white",
            "High contrast red filter",
            &[
                ("reds", f(120.0)),
                ("yellows", f(110.0)),
                ("blues", f(-10.0)),
                ("cyans", f(-30.0)),
            ],
        ),
        preset(
            "effect.black_white",
            "Infrared",
            &[
                ("reds", f(-40.0)),
                ("yellows", f(235.0)),
                ("greens", f(144.0)),
                ("cyans", f(-68.0)),
                ("blues", f(-3.0)),
                ("magentas", f(-107.0)),
            ],
        ),
        preset(
            "effect.black_white",
            "Neutral density",
            &[
                ("reds", f(53.0)),
                ("yellows", f(53.0)),
                ("greens", f(53.0)),
                ("cyans", f(53.0)),
                ("blues", f(53.0)),
                ("magentas", f(53.0)),
            ],
        ),
        preset(
            "effect.photo_filter",
            "Warming filter (85)",
            &[("color", ParamValue::Color([0.925, 0.541, 0.0, 1.0]))],
        ),
        preset(
            "effect.photo_filter",
            "Cooling filter (80)",
            &[("color", ParamValue::Color([0.0, 0.427, 1.0, 1.0]))],
        ),
        preset(
            "effect.photo_filter",
            "Sepia",
            &[("color", ParamValue::Color([0.675, 0.478, 0.2, 1.0]))],
        ),
        preset(
            "effect.channel_mixer",
            "Black and white with red filter",
            &[
                ("monochrome", b(true)),
                ("red_red", f(100.0)),
                ("red_green", f(0.0)),
                ("red_blue", f(0.0)),
            ],
        ),
        preset(
            "effect.channel_mixer",
            "Black and white infrared",
            &[
                ("monochrome", b(true)),
                ("red_red", f(-70.0)),
                ("red_green", f(200.0)),
                ("red_blue", f(-30.0)),
            ],
        ),
        preset("effect.vibrance", "Boost", &[("vibrance", f(40.0))]),
        preset(
            "effect.gradient_map",
            "Violet to orange",
            &[(
                "gradient",
                stops(&[[0.16, 0.04, 0.37, 1.0], [0.98, 0.55, 0.13, 1.0]]),
            )],
        ),
        preset(
            "effect.gradient_map",
            "Duotone blue",
            &[(
                "gradient",
                stops(&[
                    [0.05, 0.08, 0.2, 1.0],
                    [0.4, 0.6, 0.85, 1.0],
                    [0.98, 0.97, 0.92, 1.0],
                ]),
            )],
        ),
        preset(
            "effect.color_balance",
            "Warm shadows, cool highlights",
            &[
                ("shadows_cyan_red", f(20.0)),
                ("shadows_yellow_blue", f(-15.0)),
                ("highlights_cyan_red", f(-10.0)),
                ("highlights_yellow_blue", f(15.0)),
            ],
        ),
        preset(
            "effect.camera_raw",
            "Vivid",
            &[
                ("vibrance", f(35.0)),
                ("clarity", f(15.0)),
                ("contrast", f(15.0)),
            ],
        ),
        preset(
            "effect.camera_raw",
            "Soft portrait",
            &[
                ("texture", f(-30.0)),
                ("clarity", f(-15.0)),
                ("vibrance", f(10.0)),
                ("highlights", f(-20.0)),
            ],
        ),
        preset(
            "effect.camera_raw",
            "Moody",
            &[
                ("exposure", f(-0.3)),
                ("contrast", f(20.0)),
                ("saturation", f(-25.0)),
                ("shadows_hue", f(200.0)),
                ("shadows_sat", f(30.0)),
                ("vignette_amount", f(-30.0)),
            ],
        ),
        preset(
            "effect.camera_raw",
            "Film",
            &[
                ("blacks", f(30.0)),
                ("grain_amount", f(30.0)),
                ("highlights_hue", f(40.0)),
                ("highlights_sat", f(20.0)),
                ("saturation", f(-10.0)),
            ],
        ),
    ]
}

/// built in looks for colour lookup, as functions over encoded rgb
pub const LOOKS: &[&str] = &[
    "Warm",
    "Cool",
    "Teal and orange",
    "Bleach bypass",
    "Faded",
    "Night",
];

fn look(name: &str, c: [f32; 3]) -> [f32; 3] {
    let l = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    match name {
        "Warm" => [c[0] * 1.08 + 0.02, c[1] * 1.01, c[2] * 0.88],
        "Cool" => [c[0] * 0.9, c[1] * 0.99, c[2] * 1.08 + 0.02],
        "Teal and orange" => {
            let t = l.clamp(0.0, 1.0);
            [
                mix(c[0] * 0.85, c[0] * 1.15 + 0.03, t),
                mix(c[1] * 1.05, c[1] * 1.0, t),
                mix(c[2] * 1.2 + 0.04, c[2] * 0.8, t),
            ]
        }
        "Bleach bypass" => c.map(|v| {
            let s = mix(v, l, 0.6);
            let k = s - 0.5;
            0.5 + k * 1.3
        }),
        "Faded" => c.map(|v| 0.08 + v * 0.84),
        "Night" => [c[0] * 0.55, c[1] * 0.7, c[2] * 0.95 + 0.05],
        _ => c,
    }
    .map(|v| v.clamp(0.0, 1.0))
}

/// the table for a built in look, as `.cube` text so it saves like a loaded one
pub fn look_cube(name: &str) -> String {
    let n = 17;
    let mut out = format!("TITLE \"{name}\"\nLUT_3D_SIZE {n}\n");
    for bi in 0..n {
        for gi in 0..n {
            for ri in 0..n {
                let c = [ri, gi, bi].map(|v| v as f32 / (n - 1) as f32);
                let o = look(name, c);
                out.push_str(&format!("{:.6} {:.6} {:.6}\n", o[0], o[1], o[2]));
            }
        }
    }
    out
}

/// reads a `.cube` lookup table (the adobe and resolve format)
pub fn parse_cube(text: &str) -> Result<Lut3d, String> {
    let mut size = 0u32;
    let mut min = [0.0f32; 3];
    let mut max = [1.0f32; 3];
    let mut data = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("TITLE") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let head = parts.next().unwrap_or_default();
        let nums = |parts: std::str::SplitWhitespace| -> Vec<f32> {
            parts.filter_map(|p| p.parse::<f32>().ok()).collect()
        };
        match head {
            "LUT_3D_SIZE" => {
                size = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("The lookup table has a bad size line.")?;
            }
            "LUT_1D_SIZE" => {
                return Err("Only 3D lookup tables can be used here, and this one is 1D.".into());
            }
            "DOMAIN_MIN" => {
                let v = nums(parts);
                if v.len() == 3 {
                    min = [v[0], v[1], v[2]];
                }
            }
            "DOMAIN_MAX" => {
                let v = nums(parts);
                if v.len() == 3 {
                    max = [v[0], v[1], v[2]];
                }
            }
            "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {}
            _ => {
                let v: Vec<f32> = line
                    .split_whitespace()
                    .filter_map(|p| p.parse::<f32>().ok())
                    .collect();
                if v.len() == 3 {
                    data.push([v[0], v[1], v[2]]);
                }
            }
        }
    }
    if !(2..=129).contains(&size) {
        return Err("The lookup table does not say how big it is, or is too big.".into());
    }
    if data.len() != (size * size * size) as usize {
        return Err(format!(
            "The lookup table should have {} entries but has {}.",
            size * size * size,
            data.len()
        ));
    }
    // the lattice is the same as the file's (red fastest), so each sample is one entry
    let lut = Lut3d::sample(size, |c| {
        let i = |k: usize| ((c[k] * (size - 1) as f32).round() as u32).min(size - 1);
        let (r, g, b) = (i(0), i(1), i(2));
        data[(r + g * size + b * size * size) as usize]
    });
    if min != [0.0; 3] || max != [1.0; 3] {
        tracing::warn!("a lookup table's domain is not 0 to 1; it is used as if it were");
    }
    Ok(lut)
}

/// the colour transform a lookup adjustment applies in a working space. luts are made for
/// gamma encoded values, so linear documents go through srgb encoding around it.
pub fn lut_transform(table: &LutTable, working: WorkingSpace) -> Result<ColorTransform, String> {
    let lut = Arc::new(parse_cube(&table.cube)?);
    let steps = if working.encoding == Transfer::Linear {
        vec![
            ColorStep::Encode(Transfer::Srgb),
            ColorStep::Clamp,
            ColorStep::Lut(lut),
            ColorStep::Decode(Transfer::Srgb),
        ]
    } else {
        vec![ColorStep::Clamp, ColorStep::Lut(lut)]
    };
    Ok(ColorTransform { steps })
}

/// the render node for an adjustment. effects that work around a centre get the canvas's.
pub fn node(adj: &Adjustment, doc: &ImageDocument, registry: &NodeRegistry) -> Option<Node> {
    if adj.kind == "color.lut" {
        let table = adj.lut.as_ref()?;
        let t = lut_transform(table, doc.working()).ok()?;
        return Some(Node::of(vellum_gpu::nodes::LutNode).transform(t));
    }
    let mut node = registry.create(&adj.kind)?;
    for (k, v) in &adj.params {
        node.params.insert(k.clone(), v.clone());
    }
    centre_on(&mut node, doc.canvas());
    Some(node)
}

/// fills in the hidden centre and half size parameters some effects keep
pub fn centre_on(node: &mut Node, canvas: IRect) {
    let specs = node.kind.info().params;
    let c = [
        canvas.x as f32 + canvas.w as f32 / 2.0,
        canvas.y as f32 + canvas.h as f32 / 2.0,
    ];
    let e = [canvas.w as f32 / 2.0, canvas.h as f32 / 2.0];
    for spec in specs.iter().filter(|s| s.hidden) {
        match spec.id {
            "center" => {
                node.params.insert("center".into(), ParamValue::Point(c));
            }
            "extent" => {
                node.params.insert("extent".into(), ParamValue::Point(e));
            }
            _ => {}
        }
    }
}

/// the document with `id` and everything above it taken away, for working out what an
/// adjustment sees
pub fn below(doc: &ImageDocument, id: LayerId) -> ImageDocument {
    fn cut(list: &mut Vec<Layer>, id: LayerId) -> bool {
        for i in 0..list.len() {
            if list[i].id == id {
                list.truncate(i);
                return true;
            }
            if let Some(children) = list[i].children_mut()
                && cut(children, id)
            {
                list.truncate(i + 1);
                return true;
            }
        }
        false
    }
    let mut out = doc.clone();
    out.quick_mask = false;
    cut(&mut out.layers, id);
    out
}

/// up to about 200 thousand evenly spread straight colours of what the layers show
fn samples(doc: &ImageDocument) -> Result<Vec<[f32; 4]>, String> {
    let canvas = doc.canvas();
    let buf: ColorBuf = crate::flatten::colors(doc, canvas)?;
    let step = ((canvas.area() as f64 / 200_000.0).sqrt().ceil() as i32).max(1);
    let mut out = Vec::new();
    let mut y = canvas.y;
    while y < canvas.bottom() {
        let mut x = canvas.x;
        while x < canvas.right() {
            let c = buf.get(x, y);
            if c[3] > 0.5 {
                out.push(c);
            }
            x += step;
        }
        y += step;
    }
    Ok(out)
}

/// the value below which `share` of the values fall
fn percentile(values: &mut [f32], share: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let i = ((values.len() - 1) as f32 * share.clamp(0.0, 1.0)).round() as usize;
    let (_, v, _) = values.select_nth_unstable_by(i, f32::total_cmp);
    *v
}

/// how the auto buttons work, like photoshop's auto color correction options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AutoKind {
    /// stretch each channel on its own (photoshop's auto tone)
    Tone,
    /// stretch all channels together, keeping colours (auto contrast)
    Contrast,
    /// stretch each channel and pull the midtones to neutral (auto colour)
    Color,
}

const CLIP: f32 = 0.001;

/// curves that stretch the tones of `doc` the way the auto button asks
pub fn auto_curves(
    doc: &ImageDocument,
    kind: AutoKind,
) -> Result<BTreeMap<String, ParamValue>, String> {
    let px = samples(doc)?;
    if px.is_empty() {
        return Err("There is nothing below this layer to work out the tones from.".into());
    }
    let mut out = BTreeMap::new();
    let ends = |k: usize| {
        let mut v: Vec<f32> = px.iter().map(|c| c[k]).collect();
        let lo = percentile(&mut v, CLIP);
        let hi = percentile(&mut v, 1.0 - CLIP);
        (lo, hi.max(lo + 1e-3))
    };
    let stretch = |lo: f32, hi: f32| vec![[lo, 0.0], [hi, 1.0]];
    match kind {
        AutoKind::Contrast => {
            let mut v: Vec<f32> = px
                .iter()
                .map(|c| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2])
                .collect();
            let lo = percentile(&mut v, CLIP);
            let hi = percentile(&mut v, 1.0 - CLIP).max(lo + 1e-3);
            out.insert("rgb".into(), ParamValue::Curve(stretch(lo, hi)));
        }
        AutoKind::Tone | AutoKind::Color => {
            let names = ["red", "green", "blue"];
            let bounds: Vec<(f32, f32)> = (0..3).map(ends).collect();
            // auto colour also moves each channel's middle so the average midtone is grey
            let mid: Vec<f32> = if kind == AutoKind::Color {
                let mids: Vec<[f32; 3]> = px
                    .iter()
                    .map(|c| [c[0], c[1], c[2]])
                    .filter(|c| {
                        let l = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
                        (0.25..0.75).contains(&l)
                    })
                    .collect();
                let mean = |k: usize| {
                    if mids.is_empty() {
                        0.5
                    } else {
                        mids.iter().map(|c| c[k]).sum::<f32>() / mids.len() as f32
                    }
                };
                (0..3).map(mean).collect()
            } else {
                Vec::new()
            };
            for k in 0..3 {
                let (lo, hi) = bounds[k];
                let mut pts = stretch(lo, hi);
                // a nearly flat channel has no room for a middle point
                if let Some(m) = mid.get(k).filter(|_| hi - lo > 0.06) {
                    let target = mid.iter().sum::<f32>() / 3.0;
                    let at = m.clamp(lo + 0.02, hi - 0.02);
                    let to = ((target - lo) / (hi - lo)).clamp(0.05, 0.95);
                    pts.insert(1, [at, to]);
                }
                out.insert(names[k].into(), ParamValue::Curve(pts));
            }
        }
    }
    Ok(out)
}

/// a curve that spreads the tones evenly, like photoshop's equalize
pub fn equalize_curve(doc: &ImageDocument) -> Result<BTreeMap<String, ParamValue>, String> {
    let px = samples(doc)?;
    if px.is_empty() {
        return Err("There is nothing below this layer to equalize.".into());
    }
    let mut hist = [0u32; 256];
    for c in &px {
        let l = (0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]).clamp(0.0, 1.0);
        hist[(l * 255.0).round() as usize] += 1;
    }
    let total = px.len() as f32;
    let mut acc = 0.0;
    let mut cdf = [0.0f32; 256];
    for (i, h) in hist.iter().enumerate() {
        acc += *h as f32;
        cdf[i] = acc / total;
    }
    let mut pts = vec![[0.0, 0.0]];
    for i in (16..256).step_by(16) {
        pts.push([i as f32 / 255.0, cdf[i].clamp(0.0, 1.0)]);
    }
    pts.push([1.0, 1.0]);
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
    let mut out = BTreeMap::new();
    out.insert("rgb".into(), ParamValue::Curve(pts));
    Ok(out)
}

/// one click enhance: camera raw settings that stretch the tones a little, open the
/// shadows, lift dull colours and add a touch of clarity
pub fn enhance(doc: &ImageDocument) -> Result<BTreeMap<String, ParamValue>, String> {
    let px = samples(doc)?;
    if px.is_empty() {
        return Err("There is nothing below this layer to enhance.".into());
    }
    let mut lum: Vec<f32> = px
        .iter()
        .map(|c| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2])
        .collect();
    let lo = percentile(&mut lum, 0.01);
    let hi = percentile(&mut lum, 0.99);
    let median = percentile(&mut lum, 0.5);
    let sat: f32 = px
        .iter()
        .map(|c| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]))
        .sum::<f32>()
        / px.len() as f32;
    let mut p = BTreeMap::new();
    let mut set = |k: &str, v: f32| {
        p.insert(k.to_owned(), ParamValue::Float((v * 10.0).round() / 10.0));
    };
    // exposure moves the middle towards a mid grey, gently
    let exposure = ((0.45 / median.max(0.02)).log2() * 0.5).clamp(-1.0, 1.0);
    set("exposure", exposure);
    set("blacks", ((lo - 0.02) * -150.0).clamp(-40.0, 0.0));
    set("whites", ((0.98 - hi) * 150.0).clamp(0.0, 40.0));
    set(
        "shadows",
        ((0.35 - median).max(0.0) * 100.0).clamp(0.0, 35.0),
    );
    set(
        "highlights",
        ((median - 0.65).max(0.0) * -100.0).clamp(-35.0, 0.0),
    );
    set("contrast", ((0.6 - (hi - lo)) * 60.0).clamp(0.0, 25.0));
    set("vibrance", ((0.35 - sat) * 80.0).clamp(0.0, 30.0));
    set("clarity", 10.0);
    Ok(p)
}

/// match colour's statistics: `source` is the look to copy, `target` the picture changed
pub fn match_stats(
    source: &ImageDocument,
    target: &ImageDocument,
) -> Result<BTreeMap<String, ParamValue>, String> {
    let s = samples(source)?;
    let t = samples(target)?;
    if s.is_empty() || t.is_empty() {
        return Err("Both pictures need some pixels to match colours.".into());
    }
    let (sm, ss) = opponent_stats(s.iter().map(|c| [c[0], c[1], c[2]]));
    let (tm, ts) = opponent_stats(t.iter().map(|c| [c[0], c[1], c[2]]));
    let mut p = BTreeMap::new();
    p.insert("source_mean".into(), ParamValue::Color(sm));
    p.insert("source_spread".into(), ParamValue::Color(ss));
    p.insert("target_mean".into(), ParamValue::Color(tm));
    p.insert("target_spread".into(), ParamValue::Color(ts));
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Depth;

    fn gradient_doc(lo: f32, hi: f32) -> ImageDocument {
        let mut doc = ImageDocument::new("t", 64, 16, Depth::U8, Some([0.0, 0.0, 0.0, 1.0]));
        let id = doc.layers[0].id;
        let r = doc
            .layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster");
        r.update(IRect::of_size(64, 16), |x, _, px| {
            let v = lo + (hi - lo) * x as f32 / 63.0;
            px.copy_from_slice(&[v, v * 0.9, v * 0.8, 1.0]);
        });
        doc
    }

    #[test]
    fn auto_contrast_stretches_a_dull_picture() {
        let doc = gradient_doc(0.3, 0.6);
        let p = auto_curves(&doc, AutoKind::Contrast).expect("auto");
        let ParamValue::Curve(c) = &p["rgb"] else {
            panic!("no curve")
        };
        assert!(c[0][0] > 0.2 && c[0][0] < 0.32, "{c:?}");
        assert!(c[1][0] > 0.45 && c[1][0] < 0.62, "{c:?}");
        let tone = auto_curves(&doc, AutoKind::Tone).expect("tone");
        assert!(tone.contains_key("red") && tone.contains_key("blue"));
        let color = auto_curves(&doc, AutoKind::Color).expect("colour");
        let ParamValue::Curve(r) = &color["red"] else {
            panic!()
        };
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn equalize_and_enhance_give_sane_settings() {
        let doc = gradient_doc(0.1, 0.4);
        let eq = equalize_curve(&doc).expect("eq");
        let ParamValue::Curve(c) = &eq["rgb"] else {
            panic!()
        };
        assert!(c.windows(2).all(|w| w[1][1] >= w[0][1]));
        let e = enhance(&doc).expect("enhance");
        assert!(matches!(e["exposure"], ParamValue::Float(v) if v > 0.0));
    }

    #[test]
    fn cube_files_read_and_looks_round_trip() {
        let cube = look_cube("Warm");
        let lut = parse_cube(&cube).expect("parse");
        let out = lut.apply([0.5, 0.5, 0.5]);
        assert!(out[0] > out[2]);
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(parse_cube("LUT_1D_SIZE 4\n").is_err());
        let identity = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = parse_cube(identity).expect("identity");
        let c = lut.apply([0.25, 0.5, 0.75]);
        assert!((c[0] - 0.25).abs() < 1e-5 && (c[2] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn every_kind_is_a_registered_node() {
        let reg = NodeRegistry::with_builtins();
        for (kind, _) in KINDS {
            assert!(reg.get(kind).is_some(), "{kind}");
        }
        for p in presets() {
            let info = reg.get(&p.kind).expect("kind").info();
            for k in p.params.keys() {
                assert!(info.params.iter().any(|s| s.id == k), "{} {}", p.name, k);
            }
        }
    }

    #[test]
    fn below_cuts_the_layer_and_what_is_above() {
        let mut doc = gradient_doc(0.0, 1.0);
        let base = doc.layers[0].id;
        let id = doc.new_id();
        doc.layers.push(Layer::pixels(
            id,
            "top",
            crate::raster::Raster::rgba(Depth::U8),
        ));
        assert_eq!(below(&doc, id).layers.len(), 1);
        assert_eq!(below(&doc, base).layers.len(), 0);
    }
}
