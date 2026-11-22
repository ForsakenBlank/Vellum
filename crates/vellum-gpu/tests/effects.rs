//! every adjustment and filter from phase 6: golden images in `tests/golden/effects/` at the
//! repo root, and the gpu against the cpu in linear and srgb working spaces at two zoom
//! levels. run with `VELLUM_UPDATE_GOLDEN=1` to write new references after a deliberate
//! change, then look at them (`all.png` is a contact sheet) before committing.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{assert_close, gpu_engine, region};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::params::GradientStop;
use vellum_gpu::{
    BackendChoice, CpuImage, Engine, Graph, IRect, ImageSource, Node, NodeId, NodeRegistry,
    ParamValue, Precision, ProceduralSource, WorkingSpace,
};

const W: u32 = 96;
const H: u32 = 72;

/// a small photo-like picture: a sky gradient, a warm disc, fine stripes for detail,
/// a dark corner and a soft see through edge on the right
fn photo() -> Arc<dyn ImageSource> {
    Arc::new(ProceduralSource::new(W, H, |x, y| {
        let (u, v) = (x / W as f32, y / H as f32);
        let mut c = [0.25 + 0.4 * v, 0.45 + 0.3 * v, 0.85 - 0.3 * v];
        let d = (x - 34.0).hypot(y - 38.0);
        if d < 20.0 {
            let t = (20.0 - d).min(1.5) / 1.5;
            c = [
                c[0] + (0.95 - c[0]) * t,
                c[1] + (0.55 - c[1]) * t,
                c[2] + (0.2 - c[2]) * t,
            ];
        }
        if x > 60.0 && y > 40.0 {
            let s = if ((x / 3.0) as i32 + (y / 3.0) as i32) % 2 == 0 {
                0.15
            } else {
                0.0
            };
            c = [0.1 + s, 0.12 + s, 0.1 + s * 0.5];
        }
        let a = if u > 0.85 {
            1.0 - (u - 0.85) / 0.15 * 0.7
        } else {
            1.0
        };
        [c[0], c[1], c[2], a]
    }))
}

fn grad(stops: &[([f32; 4], f32)]) -> ParamValue {
    ParamValue::Gradient(
        stops
            .iter()
            .map(|(color, offset)| GradientStop {
                offset: *offset,
                color: *color,
            })
            .collect(),
    )
}

type Params = Vec<(&'static str, ParamValue)>;

/// each case is a node id and the parameters that make it do something visible
fn cases() -> Vec<(&'static str, &'static str, Params)> {
    let f = |v: f32| ParamValue::Float(v);
    let b = |v: bool| ParamValue::Bool(v);
    let c = |s: &str| ParamValue::Choice(s.into());
    let pt = |x: f32, y: f32| ParamValue::Point([x, y]);
    let lens = || vec![("center", pt(48.0, 36.0)), ("extent", pt(48.0, 36.0))];
    let raw = |extra: Params| {
        let mut p = lens();
        p.extend(extra);
        p
    };
    vec![
        (
            "curves",
            "effect.curves",
            vec![
                (
                    "rgb",
                    ParamValue::Curve(vec![[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]]),
                ),
                ("blue", ParamValue::Curve(vec![[0.0, 0.1], [1.0, 0.9]])),
            ],
        ),
        (
            "brightness_contrast",
            "effect.brightness_contrast",
            vec![("brightness", f(40.0)), ("contrast", f(50.0))],
        ),
        (
            "brightness_contrast_legacy",
            "effect.brightness_contrast",
            vec![
                ("brightness", f(-30.0)),
                ("contrast", f(30.0)),
                ("legacy", b(true)),
            ],
        ),
        (
            "levels",
            "effect.levels",
            vec![("input_black", f(20.0)), ("gamma", f(1.4))],
        ),
        (
            "hue_saturation",
            "effect.hue_saturation",
            vec![("hue", f(60.0)), ("saturation", f(30.0))],
        ),
        ("invert", "effect.invert", vec![]),
        (
            "posterize",
            "effect.posterize",
            vec![("levels", ParamValue::Int(4))],
        ),
        ("threshold", "effect.threshold", vec![("level", f(110.0))]),
        (
            "exposure",
            "effect.exposure",
            vec![
                ("exposure", f(0.8)),
                ("offset", f(-0.02)),
                ("gamma", f(1.1)),
            ],
        ),
        (
            "vibrance",
            "effect.vibrance",
            vec![("vibrance", f(70.0)), ("saturation", f(-10.0))],
        ),
        (
            "color_balance",
            "effect.color_balance",
            vec![
                ("shadows_cyan_red", f(-30.0)),
                ("midtones_yellow_blue", f(40.0)),
                ("highlights_magenta_green", f(25.0)),
            ],
        ),
        (
            "black_white",
            "effect.black_white",
            vec![("reds", f(80.0)), ("blues", f(-20.0))],
        ),
        (
            "black_white_tint",
            "effect.black_white",
            vec![("tint", b(true)), ("tint_saturation", f(40.0))],
        ),
        (
            "photo_filter",
            "effect.photo_filter",
            vec![("density", f(60.0))],
        ),
        (
            "channel_mixer",
            "effect.channel_mixer",
            vec![
                ("red_green", f(40.0)),
                ("red_red", f(70.0)),
                ("blue_constant", f(10.0)),
            ],
        ),
        (
            "channel_mixer_mono",
            "effect.channel_mixer",
            vec![
                ("monochrome", b(true)),
                ("red_red", f(40.0)),
                ("red_green", f(40.0)),
                ("red_blue", f(20.0)),
            ],
        ),
        (
            "gradient_map",
            "effect.gradient_map",
            vec![(
                "gradient",
                grad(&[
                    ([0.1, 0.0, 0.3, 1.0], 0.0),
                    ([1.0, 0.5, 0.1, 1.0], 0.6),
                    ([1.0, 1.0, 0.8, 1.0], 1.0),
                ]),
            )],
        ),
        (
            "selective_color",
            "effect.selective_color",
            vec![
                ("reds_cyan", f(-60.0)),
                ("blues_yellow", f(50.0)),
                ("neutrals_black", f(20.0)),
            ],
        ),
        (
            "selective_color_absolute",
            "effect.selective_color",
            vec![
                ("method", c("absolute")),
                ("yellows_magenta", f(40.0)),
                ("blacks_black", f(-30.0)),
            ],
        ),
        (
            "match_color",
            "effect.match_color",
            vec![
                ("source_mean", ParamValue::Color([0.55, -0.15, 0.2, 0.0])),
                ("source_spread", ParamValue::Color([0.2, 0.08, 0.1, 0.0])),
                ("target_mean", ParamValue::Color([0.5, 0.1, -0.1, 0.0])),
                ("target_spread", ParamValue::Color([0.25, 0.12, 0.12, 0.0])),
                ("fade", f(20.0)),
            ],
        ),
        (
            "replace_color",
            "effect.replace_color",
            vec![
                ("color", ParamValue::Color([0.95, 0.55, 0.2, 1.0])),
                ("hue", f(-120.0)),
                ("fuzziness", f(80.0)),
            ],
        ),
        (
            "shadows_highlights",
            "effect.shadows_highlights",
            vec![
                ("shadows", f(60.0)),
                ("highlights", f(30.0)),
                ("radius", f(8.0)),
            ],
        ),
        (
            "hdr_toning",
            "effect.hdr_toning",
            vec![
                ("radius", f(10.0)),
                ("strength", f(0.7)),
                ("detail", f(80.0)),
            ],
        ),
        (
            "camera_raw_basic",
            "effect.camera_raw",
            raw(vec![
                ("temperature", f(30.0)),
                ("exposure", f(0.3)),
                ("contrast", f(25.0)),
                ("highlights", f(-40.0)),
                ("shadows", f(40.0)),
                ("vibrance", f(30.0)),
            ]),
        ),
        (
            "camera_raw_presence",
            "effect.camera_raw",
            raw(vec![
                ("texture", f(50.0)),
                ("clarity", f(40.0)),
                ("dehaze", f(30.0)),
                ("whites", f(20.0)),
                ("blacks", f(-20.0)),
            ]),
        ),
        (
            "camera_raw_color",
            "effect.camera_raw",
            raw(vec![
                ("orange_hue", f(40.0)),
                ("blue_saturation", f(-60.0)),
                ("blue_luminance", f(30.0)),
                ("shadows_sat", f(50.0)),
                ("highlights_sat", f(40.0)),
                ("balance", f(20.0)),
                ("tint", f(-20.0)),
            ]),
        ),
        (
            "camera_raw_detail",
            "effect.camera_raw",
            raw(vec![
                ("sharpen_amount", f(80.0)),
                ("sharpen_masking", f(30.0)),
                ("noise_luminance", f(40.0)),
                ("noise_color", f(50.0)),
            ]),
        ),
        (
            "camera_raw_optics",
            "effect.camera_raw",
            raw(vec![
                ("distortion", f(-40.0)),
                ("chromatic_aberration", f(60.0)),
                ("vignetting", f(30.0)),
            ]),
        ),
        (
            "camera_raw_effects",
            "effect.camera_raw",
            raw(vec![
                ("vignette_amount", f(-60.0)),
                ("vignette_roundness", f(30.0)),
                ("grain_amount", f(50.0)),
                ("grain_size", f(40.0)),
            ]),
        ),
    ]
    .into_iter()
    .chain(filter_cases())
    .collect()
}

/// filters from the filter gallery
fn filter_cases() -> Vec<(&'static str, &'static str, Params)> {
    let f = |v: f32| ParamValue::Float(v);
    let b = |v: bool| ParamValue::Bool(v);
    let c = |s: &str| ParamValue::Choice(s.into());
    let pt = |x: f32, y: f32| ParamValue::Point([x, y]);
    vec![
        (
            "gaussian_blur",
            "effect.gaussian_blur",
            vec![("radius", f(3.0))],
        ),
        (
            "motion_blur",
            "effect.motion_blur",
            vec![("angle", f(30.0)), ("distance", f(12.0))],
        ),
        (
            "radial_blur_spin",
            "effect.radial_blur",
            vec![("center", pt(34.0, 38.0)), ("amount", f(25.0))],
        ),
        (
            "radial_blur_zoom",
            "effect.radial_blur",
            vec![
                ("mode", c("zoom")),
                ("center", pt(48.0, 36.0)),
                ("amount", f(30.0)),
            ],
        ),
        (
            "lens_blur",
            "effect.lens_blur",
            vec![
                ("radius", f(5.0)),
                ("brightness", f(60.0)),
                ("threshold", f(150.0)),
            ],
        ),
        (
            "tilt_shift",
            "effect.focus_blur",
            vec![
                ("center", pt(48.0, 38.0)),
                ("blur", f(6.0)),
                ("focus", f(10.0)),
                ("transition", f(20.0)),
                ("angle", f(10.0)),
            ],
        ),
        (
            "iris_blur",
            "effect.focus_blur",
            vec![
                ("mode", c("iris")),
                ("center", pt(34.0, 38.0)),
                ("blur", f(6.0)),
                ("focus", f(30.0)),
                ("transition", f(15.0)),
                ("aspect", f(1.5)),
            ],
        ),
        (
            "field_blur",
            "effect.focus_blur",
            vec![
                ("mode", c("field")),
                ("center", pt(0.0, 0.0)),
                ("blur", f(0.0)),
                ("end", pt(96.0, 72.0)),
                ("end_blur", f(6.0)),
            ],
        ),
        (
            "surface_blur",
            "effect.surface_blur",
            vec![("radius", f(4.0)), ("threshold", f(40.0))],
        ),
        (
            "sharpen",
            "effect.sharpen",
            vec![("amount", f(150.0)), ("radius", f(1.5))],
        ),
        (
            "smart_sharpen",
            "effect.smart_sharpen",
            vec![
                ("amount", f(200.0)),
                ("radius", f(1.5)),
                ("highlights_fade", f(40.0)),
            ],
        ),
        ("high_pass", "effect.high_pass", vec![("radius", f(3.0))]),
        (
            "reduce_noise",
            "effect.reduce_noise",
            vec![("strength", f(8.0)), ("color", f(80.0))],
        ),
        ("add_noise", "effect.add_noise", vec![("amount", f(20.0))]),
        (
            "add_noise_gaussian_mono",
            "effect.add_noise",
            vec![
                ("amount", f(30.0)),
                ("distribution", c("gaussian")),
                ("monochromatic", b(true)),
            ],
        ),
        (
            "film_grain",
            "effect.film_grain",
            vec![("amount", f(60.0)), ("size", f(2.0))],
        ),
        (
            "wave",
            "effect.wave",
            vec![("wavelength", f(30.0)), ("amplitude", f(4.0))],
        ),
        (
            "wave_square",
            "effect.wave",
            vec![
                ("kind", c("square")),
                ("wavelength", f(24.0)),
                ("amplitude", f(3.0)),
                ("vertical", f(0.0)),
            ],
        ),
        (
            "ripple",
            "effect.ripple",
            vec![("amount", f(200.0)), ("size", c("small"))],
        ),
        (
            "twirl",
            "effect.twirl",
            vec![
                ("center", pt(34.0, 38.0)),
                ("radius", f(30.0)),
                ("angle", f(180.0)),
            ],
        ),
        (
            "polar_to_polar",
            "effect.polar",
            vec![("center", pt(48.0, 36.0)), ("extent", pt(48.0, 36.0))],
        ),
        (
            "polar_to_rect",
            "effect.polar",
            vec![
                ("mode", c("to_rect")),
                ("center", pt(48.0, 36.0)),
                ("extent", pt(48.0, 36.0)),
            ],
        ),
        (
            "displace",
            "effect.displace",
            vec![("horizontal", f(8.0)), ("vertical", f(4.0))],
        ),
        (
            "chromatic_aberration",
            "effect.chromatic_aberration",
            vec![
                ("amount", f(4.0)),
                ("center", pt(48.0, 36.0)),
                ("extent", pt(48.0, 36.0)),
            ],
        ),
        (
            "glitch",
            "effect.glitch",
            vec![
                ("amount", f(60.0)),
                ("band", f(6.0)),
                ("shift", f(10.0)),
                ("split", f(2.0)),
            ],
        ),
        ("emboss", "effect.emboss", vec![("height", f(2.0))]),
        ("find_edges", "effect.find_edges", vec![]),
        (
            "glowing_edges",
            "effect.glowing_edges",
            vec![("width", f(1.5))],
        ),
        ("oil_paint", "effect.oil_paint", vec![("radius", f(4.0))]),
        ("mosaic", "effect.mosaic", vec![("cell", f(8.0))]),
        ("halftone", "effect.halftone", vec![("cell", f(6.0))]),
        (
            "halftone_mono",
            "effect.halftone",
            vec![("cell", f(5.0)), ("mode", c("mono"))],
        ),
        ("clouds", "effect.clouds", vec![("scale", f(40.0))]),
        (
            "lens_flare",
            "effect.lens_flare",
            vec![
                ("position", pt(70.0, 18.0)),
                ("size", f(14.0)),
                ("center", pt(48.0, 36.0)),
            ],
        ),
        (
            "lighting_spot",
            "effect.lighting",
            vec![
                ("position", pt(20.0, 10.0)),
                ("angle", f(40.0)),
                ("reach", f(70.0)),
                ("height", f(30.0)),
            ],
        ),
        (
            "lighting_infinite",
            "effect.lighting",
            vec![
                ("kind", c("infinite")),
                ("ambience", f(40.0)),
                ("height", f(40.0)),
            ],
        ),
    ]
}

fn scene(id: &str, params: &Params) -> (Graph, NodeId) {
    let reg = NodeRegistry::with_builtins();
    let mut g = Graph::new();
    let pic = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(photo()));
    let mut node = reg.create(id).unwrap_or_else(|| panic!("no node {id}"));
    for (k, v) in params {
        node.params.insert((*k).to_owned(), v.clone());
    }
    if !node.inputs.is_empty() {
        node.inputs[0] = Some(pic);
    }
    let top = g.add(node);
    (g, top)
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/effects")
}

fn to_png16(img: &CpuImage) -> Vec<u16> {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    img.data
        .iter()
        .flat_map(|c| {
            let s = if c[3] > 0.0 {
                [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
            } else {
                [0.0; 4]
            };
            s.map(q)
        })
        .collect()
}

fn save(path: &PathBuf, data: Vec<u16>, w: u32, h: u32) {
    let img: image::ImageBuffer<image::Rgba<u16>, Vec<u16>> =
        image::ImageBuffer::from_raw(w, h, data).expect("buffer size");
    img.save(path).expect("write golden image");
}

#[test]
fn every_effect_matches_its_golden_image() {
    let update = std::env::var_os("VELLUM_UPDATE_GOLDEN").is_some();
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("golden folder");
    let cpu = Engine::cpu_only();
    let cases = cases();
    let cols = 6u32;
    let rows = (cases.len() as u32).div_ceil(cols);
    let mut sheet = vec![0u16; (W * cols * H * rows * 4) as usize];
    let mut missing = Vec::new();
    let mut unchanged = Vec::new();
    let original = {
        let (g, top) = scene("effect.levels", &Vec::new());
        cpu.render_image(&g, &RenderRequest::new(top, IRect::of_size(W, H)))
            .expect("original")
    };
    for (i, (name, id, params)) in cases.iter().enumerate() {
        let (g, top) = scene(id, params);
        let req = RenderRequest::new(top, IRect::of_size(W, H))
            .working(WorkingSpace::SRGB)
            .precision(Precision::F32)
            .backend(BackendChoice::Cpu);
        let out = cpu.render_image(&g, &req).expect(name);
        if out == original {
            unchanged.push(*name);
        }
        let stored = to_png16(&out);
        let (cx, cy) = ((i as u32 % cols) * W, (i as u32 / cols) * H);
        for y in 0..H {
            for x in 0..W {
                let src = ((y * W + x) * 4) as usize;
                let dst = (((cy + y) * W * cols + cx + x) * 4) as usize;
                sheet[dst..dst + 4].copy_from_slice(&stored[src..src + 4]);
            }
        }
        let path = dir.join(format!("{name}.png"));
        if update {
            save(&path, stored, W, H);
        } else if !path.exists() {
            missing.push(*name);
        } else {
            let golden = image::open(&path).expect("read").to_rgba16().into_raw();
            let worst = golden
                .iter()
                .zip(&stored)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 3,
                "{name} no longer matches its golden image: off by {worst}/65535"
            );
        }
    }
    if update {
        save(&dir.join("all.png"), sheet, W * cols, H * rows);
    }
    assert!(
        unchanged.is_empty(),
        "these did nothing to the picture: {unchanged:?}"
    );
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}

#[test]
fn every_effect_agrees_between_cpu_and_gpu() {
    let Some(gpu) = gpu_engine() else { return };
    let cpu = Engine::cpu_only();
    let mut failures = Vec::new();
    for (name, id, params) in cases() {
        let (g, top) = scene(id, &params);
        for working in [WorkingSpace::SRGB, WorkingSpace::LINEAR_SRGB] {
            for level in [0, 1] {
                let req = RenderRequest::new(top, region(-6, -4, W + 12, H + 8))
                    .level(level)
                    .working(working)
                    .precision(Precision::F32);
                let a = cpu
                    .render_image(&g, &req.backend(BackendChoice::Cpu))
                    .expect(name);
                let b = gpu
                    .render_image(&g, &req.backend(BackendChoice::Gpu))
                    .expect(name);
                let label = format!("{name} ({working:?}, level {level})");
                if let Err(e) = std::panic::catch_unwind(|| assert_close(&label, &a, &b, 2e-3)) {
                    failures.push(
                        e.downcast_ref::<String>()
                            .cloned()
                            .unwrap_or_else(|| label.clone()),
                    );
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_registered_effect_has_a_case() {
    let reg = NodeRegistry::with_builtins();
    let covered: Vec<&str> = cases().iter().map(|c| c.1).collect();
    let phase4 = [
        "source.image",
        "source.solid",
        "source.gradient",
        "blend",
        "mask",
        "transform",
        "color.convert",
        "color.lut",
        "output",
        "effect.gaussian_blur",
        "effect.sharpen",
    ];
    let missing: Vec<&str> = reg
        .infos()
        .iter()
        .map(|i| i.id)
        .filter(|id| !covered.contains(id) && !phase4.contains(id) && !id.starts_with("style."))
        .collect();
    assert!(
        missing.is_empty(),
        "effects without a golden case: {missing:?}"
    );
}

/// a soft edged disc and a square, the shape layer styles are drawn from
fn shape() -> Arc<dyn ImageSource> {
    Arc::new(ProceduralSource::new(W, H, |x, y| {
        let disc = (22.0 - (x - 36.0).hypot(y - 36.0)).clamp(0.0, 1.0);
        let square = if (60.0..84.0).contains(&x) && (20.0..52.0).contains(&y) {
            1.0
        } else {
            0.0
        };
        let a = disc.max(square);
        [0.9, 0.5, 0.2, a]
    }))
}

/// graphs that wire the layer style pieces the way the image editor does
fn style_scenes() -> Vec<(&'static str, Graph, NodeId)> {
    let reg = NodeRegistry::with_builtins();
    let node = |id: &str, params: &[(&str, ParamValue)]| {
        let mut n = reg.create(id).expect(id);
        for (k, v) in params {
            n.params.insert((*k).to_owned(), v.clone());
        }
        n
    };
    let f = |v: f32| ParamValue::Float(v);
    let c = |s: &str| ParamValue::Choice(s.into());
    let mut out = Vec::new();
    let mut effect = |name: &'static str, kind: &str, blur: f32, extra: &[(&str, ParamValue)]| {
        let mut g = Graph::new();
        let layer = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(shape()));
        let blurred = g.add(
            node(
                "effect.gaussian_blur",
                &[("radius", f(blur)), ("edges", c("transparent"))],
            )
            .input(0, layer),
        );
        let mut params = vec![("kind", c(kind))];
        params.extend(extra.iter().cloned());
        let top = g.add(
            node("style.effect", &params)
                .input(0, blurred)
                .input(1, layer),
        );
        out.push((name, g, top));
    };
    let red = ParamValue::Color([0.8, 0.1, 0.1, 1.0]);
    effect(
        "style_drop_shadow",
        "shadow",
        2.0,
        &[
            ("offset", ParamValue::Point([4.0, 5.0])),
            ("spread", f(20.0)),
        ],
    );
    effect(
        "style_inner_shadow",
        "inner_shadow",
        2.0,
        &[("offset", ParamValue::Point([3.0, 3.0]))],
    );
    effect(
        "style_outer_glow",
        "outer_glow",
        3.0,
        &[
            ("color", ParamValue::Color([1.0, 0.9, 0.3, 1.0])),
            ("spread", f(30.0)),
        ],
    );
    effect(
        "style_inner_glow_edge",
        "inner_glow_edge",
        3.0,
        &[("color", red.clone())],
    );
    effect(
        "style_inner_glow_center",
        "inner_glow_center",
        3.0,
        &[("color", red.clone())],
    );
    effect(
        "style_satin",
        "satin",
        3.0,
        &[("offset", ParamValue::Point([4.0, 4.0]))],
    );
    effect(
        "style_bevel_highlight",
        "bevel_highlight",
        2.5,
        &[("color", ParamValue::Color([1.0, 1.0, 1.0, 1.0]))],
    );
    effect(
        "style_bevel_shadow",
        "bevel_shadow",
        2.5,
        &[("down", ParamValue::Bool(true))],
    );

    let mut g = Graph::new();
    let layer = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(shape()));
    let grown = g.add(node("style.morph", &[("radius", f(3.0))]).input(0, layer));
    let top = g.add(
        node(
            "style.effect",
            &[("kind", c("stroke")), ("color", red.clone())],
        )
        .input(0, grown)
        .input(1, layer)
        .input(2, layer),
    );
    out.push(("style_stroke_outside", g, top));

    let mut g = Graph::new();
    let layer = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(shape()));
    let shrunk =
        g.add(node("style.morph", &[("radius", f(4.0)), ("mode", c("shrink"))]).input(0, layer));
    out.push(("style_morph_shrink", g, shrunk));

    let mut g = Graph::new();
    let pic = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(photo()));
    let inverted = g.add(node("effect.invert", &[]).input(0, pic));
    let mask = g.add(Node::of(vellum_gpu::nodes::ImageNode).source(shape()));
    let top = g.add(
        node(
            "mix",
            &[
                ("amount", f(80.0)),
                ("mask_channel", c("alpha")),
                ("masked", ParamValue::Bool(true)),
            ],
        )
        .input(0, pic)
        .input(1, inverted)
        .input(2, mask),
    );
    out.push(("mix_through_mask", g, top));

    let mut g = Graph::new();
    let tile: Vec<[f32; 4]> = (0..16)
        .map(|i| {
            let (x, y) = (i % 4, i / 4);
            if (x + y) % 2 == 0 {
                [0.9, 0.2, 0.3, 1.0]
            } else {
                [0.2, 0.3, 0.9, 0.6]
            }
        })
        .collect();
    let top = g.add(
        node(
            "source.pattern",
            &[
                ("scale", f(200.0)),
                ("offset", ParamValue::Point([3.0, 1.0])),
            ],
        )
        .data(vellum_gpu::nodes::style::pattern_data(4, 4, &tile)),
    );
    out.push(("pattern", g, top));
    out
}

#[test]
fn style_pieces_match_their_goldens_and_agree_on_the_gpu() {
    let update = std::env::var_os("VELLUM_UPDATE_GOLDEN").is_some();
    let dir = golden_dir();
    let cpu = Engine::cpu_only();
    let gpu = gpu_engine();
    for (name, g, top) in style_scenes() {
        let req = RenderRequest::new(top, IRect::of_size(W, H))
            .working(WorkingSpace::SRGB)
            .precision(Precision::F32);
        let out = cpu
            .render_image(&g, &req.backend(BackendChoice::Cpu))
            .expect(name);
        assert!(out.data.iter().any(|p| p[3] > 0.01), "{name} drew nothing");
        let stored = to_png16(&out);
        let path = dir.join(format!("{name}.png"));
        if update {
            save(&path, stored, W, H);
        } else {
            let golden = image::open(&path).expect("golden").to_rgba16().into_raw();
            let worst = golden
                .iter()
                .zip(&stored)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= 3, "{name} off by {worst}/65535");
        }
        if let Some(gpu) = &gpu {
            for working in [WorkingSpace::SRGB, WorkingSpace::LINEAR_SRGB] {
                for level in [0, 1] {
                    let req = RenderRequest::new(top, region(-6, -4, W + 12, H + 8))
                        .level(level)
                        .working(working)
                        .precision(Precision::F32);
                    let a = cpu
                        .render_image(&g, &req.backend(BackendChoice::Cpu))
                        .expect(name);
                    let b = gpu
                        .render_image(&g, &req.backend(BackendChoice::Gpu))
                        .expect(name);
                    assert_close(
                        &format!("{name} ({working:?}, level {level})"),
                        &a,
                        &b,
                        2e-3,
                    );
                }
            }
        }
    }
}
