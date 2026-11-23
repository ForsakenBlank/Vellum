//! golden images for layer compositing and selection maths, in `tests/golden/image/` at
//! the repo root.
//!
//! compositing scenes are rendered on the processor in 32 bit float and stored as 16 bit
//! pngs; the graphics card, when there is one, must agree. selections are stored as 8 bit
//! grey pngs, which is how selections are kept. run with `VELLUM_UPDATE_GOLDEN=1` to write
//! new references after a deliberate change, then look at them before committing.

use std::path::PathBuf;

use vellum_gpu::engine::RenderRequest;
use vellum_gpu::gpu::GpuOptions;
use vellum_gpu::{
    Backend, BackendChoice, BlendMode, CpuImage, Engine, EngineOptions, IRect, Precision,
};
use vellum_image::canvas::{Crop, ImageSize};
use vellum_image::commands::Edit;
use vellum_image::compose::{Preview, TransformPreview, build};
use vellum_image::document::{Content, ImageDocument, Layer, LayerMask};
use vellum_image::raster::{Depth, Raster};
use vellum_image::selection::{self, ColorBuf, Combine, GrayBuf, Shape};
use vellum_image::transform::{Interpolation, Transform, TransformSpec, TransformTarget};

const SIZE: u32 = 64;

fn dir(kind: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/image")
        .join(kind)
}

fn update() -> bool {
    std::env::var_os("VELLUM_UPDATE_GOLDEN").is_some()
}

fn gpu_engine() -> Option<Engine> {
    let engine = Engine::new(EngineOptions {
        gpu: GpuOptions::allowing_software(),
        ..Default::default()
    });
    if engine.status().backend == Backend::Gpu {
        return Some(engine);
    }
    if std::env::var("VELLUM_REQUIRE_GPU").is_ok_and(|v| !v.is_empty()) {
        panic!("no gpu adapter, and VELLUM_REQUIRE_GPU is set");
    }
    None
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match (i as i32).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn pixels(depth: Depth, f: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Raster {
    let mut r = Raster::rgba(depth);
    r.update(IRect::of_size(SIZE, SIZE), |x, y, px| {
        px.copy_from_slice(&f(x as f32 + 0.5, y as f32 + 0.5));
    });
    r
}

fn layer(doc: &mut ImageDocument, name: &str, f: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Layer {
    let id = doc.new_id();
    Layer::pixels(id, name, pixels(doc.depth, f))
}

fn backdrop(x: f32, y: f32) -> [f32; 4] {
    let n = SIZE as f32;
    let c = hsv(x / n, 0.7, 0.3 + 0.7 * (1.0 - y / n));
    [c[0], c[1], c[2], 1.0]
}

fn disc(cx: f32, cy: f32, r: f32, c: [f32; 3]) -> impl Fn(f32, f32) -> [f32; 4] + Sync {
    move |x, y| {
        let a = (r - (x - cx).hypot(y - cy) + 0.5).clamp(0.0, 1.0);
        [c[0], c[1], c[2], a]
    }
}

fn gray_mask(depth: Depth, f: impl Fn(f32, f32) -> f32 + Sync) -> LayerMask {
    let mut r = Raster::gray(depth, 1.0);
    r.update(IRect::of_size(SIZE, SIZE), |x, y, v| {
        v[0] = f(x as f32 + 0.5, y as f32 + 0.5)
    });
    LayerMask {
        raster: r,
        enabled: true,
        linked: true,
    }
}

fn scenes() -> Vec<(&'static str, ImageDocument)> {
    let n = SIZE as f32;
    let mut out = Vec::new();

    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
    let bg = layer(&mut doc, "bg", backdrop);
    let mut mul = layer(
        &mut doc,
        "multiply",
        disc(24.0, 28.0, 18.0, [0.9, 0.5, 0.2]),
    );
    mul.blend = BlendMode::Multiply;
    mul.opacity = 0.7;
    let mut scr = layer(&mut doc, "screen", disc(40.0, 36.0, 18.0, [0.2, 0.4, 0.9]));
    scr.blend = BlendMode::Screen;
    scr.fill = 0.5;
    doc.layers = vec![bg, mul, scr];
    out.push(("blend_opacity_fill", doc));

    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
    let bg = layer(&mut doc, "bg", backdrop);
    let mut top = layer(&mut doc, "top", move |_, y| {
        let c = hsv(0.5 + y / n * 0.5, 0.8, 0.9);
        [c[0], c[1], c[2], 1.0]
    });
    top.mask = Some(gray_mask(Depth::U16, move |x, _| {
        (x / n * 1.4 - 0.2).clamp(0.0, 1.0)
    }));
    top.blend = BlendMode::Overlay;
    doc.layers = vec![bg, top];
    out.push(("layer_mask", doc));

    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
    let bg = layer(&mut doc, "bg", backdrop);
    let mut base = layer(&mut doc, "base", disc(32.0, 32.0, 22.0, [1.0, 1.0, 1.0]));
    base.fill = 0.6;
    let mut stripes = layer(&mut doc, "stripes", |x, _| {
        let on = (x / 6.0).floor() as i32 % 2 == 0;
        if on { [0.1, 0.6, 0.3, 1.0] } else { [0.0; 4] }
    });
    stripes.clipped = true;
    let mut tint = layer(&mut doc, "tint", |_, y| [0.8, 0.2, 0.6, y / SIZE as f32]);
    tint.clipped = true;
    tint.blend = BlendMode::Multiply;
    doc.layers = vec![bg, base, stripes, tint];
    out.push(("clipping_group", doc));

    for (name, pass_through) in [("group_pass_through", true), ("group_isolated", false)] {
        let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
        let bg = layer(&mut doc, "bg", backdrop);
        let mut a = layer(&mut doc, "a", disc(24.0, 32.0, 16.0, [0.95, 0.85, 0.2]));
        a.blend = BlendMode::Multiply;
        let mut b = layer(&mut doc, "b", disc(40.0, 32.0, 16.0, [0.2, 0.7, 0.95]));
        b.blend = BlendMode::Difference;
        let gid = doc.new_id();
        let mut g = Layer::group(gid, "g", vec![a, b]);
        g.opacity = 0.8;
        if let Content::Group {
            pass_through: p, ..
        } = &mut g.content
        {
            *p = pass_through;
        }
        g.mask = Some(gray_mask(Depth::U16, move |_, y| 1.0 - (y / n) * 0.7));
        doc.layers = vec![bg, g];
        out.push((name, doc));
    }

    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
    let bg = layer(&mut doc, "bg", backdrop);
    let mut hidden_base = layer(&mut doc, "hidden", disc(32.0, 32.0, 20.0, [1.0, 0.0, 0.0]));
    hidden_base.visible = false;
    let mut clipped = layer(&mut doc, "clipped to hidden", |_, _| [0.0, 1.0, 0.0, 1.0]);
    clipped.clipped = true;
    let empty_id = doc.new_id();
    let empty = Layer::pixels(empty_id, "empty", Raster::rgba(Depth::U16));
    let mut on_empty = layer(&mut doc, "clipped to empty", |_, _| [0.0, 0.0, 1.0, 1.0]);
    on_empty.clipped = true;
    let mut soft = layer(&mut doc, "soft", disc(20.0, 44.0, 12.0, [1.0, 1.0, 1.0]));
    soft.opacity = 0.5;
    doc.layers = vec![bg, hidden_base, clipped, empty, on_empty, soft];
    out.push(("hidden_and_empty", doc));

    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::F32);
    let bg = layer(&mut doc, "bg", |x, y| {
        let c = hsv(x / SIZE as f32, 0.6, 0.2 + y / SIZE as f32);
        [c[0] * c[0], c[1] * c[1], c[2] * c[2], 1.0]
    });
    let mut glow = layer(&mut doc, "glow", disc(32.0, 32.0, 24.0, [2.0, 1.5, 0.8]));
    glow.blend = BlendMode::LinearDodge;
    glow.opacity = 0.6;
    doc.layers = vec![bg, glow];
    out.push(("depth32_linear", doc));
    out
}

fn render(engine: &Engine, doc: &ImageDocument, backend: BackendChoice) -> CpuImage {
    let (graph, node) = build(doc, &Preview::default());
    let req = RenderRequest::new(node, doc.canvas())
        .working(doc.working())
        .precision(Precision::F32)
        .backend(backend);
    engine.render_image(&graph, &req).expect("render")
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

#[test]
fn layer_compositing_matches_golden_images() {
    let out = dir("compose");
    std::fs::create_dir_all(&out).expect("golden folder");
    let cpu = Engine::cpu_only();
    let gpu = gpu_engine();
    let mut missing = Vec::new();
    for (name, doc) in scenes() {
        let reference = render(&cpu, &doc, BackendChoice::Cpu);
        let stored = to_png16(&reference);
        let path = out.join(format!("{name}.png"));
        if update() {
            let img: image::ImageBuffer<image::Rgba<u16>, Vec<u16>> =
                image::ImageBuffer::from_raw(SIZE, SIZE, stored.clone()).expect("size");
            img.save(&path).expect("write golden");
        } else if !path.exists() {
            missing.push(name);
            continue;
        } else {
            let golden = image::open(&path)
                .expect("read golden")
                .to_rgba16()
                .into_raw();
            let worst = golden
                .iter()
                .zip(&stored)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 3,
                "{name} no longer matches {}: off by {worst}/65535",
                path.display()
            );
        }
        if let Some(gpu) = &gpu {
            let on_gpu = render(gpu, &doc, BackendChoice::Gpu);
            let worst = to_png16(&on_gpu)
                .iter()
                .zip(&stored)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= 160, "{name} on the gpu differs by {worst}/65535");
        }
    }
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}

#[test]
fn compositing_follows_photoshop_rules() {
    let cpu = Engine::cpu_only();
    let scenes = scenes();
    let find = |n: &str| &scenes.iter().find(|(name, _)| *name == n).expect("scene").1;
    let at = |img: &CpuImage, x: i32, y: i32| img.get(x, y);

    // a clipped layer only shows where its base has pixels
    let clip = render(&cpu, find("clipping_group"), BackendChoice::Cpu);
    let bare = {
        let mut d = find("clipping_group").clone();
        d.layers.truncate(1);
        render(&cpu, &d, BackendChoice::Cpu)
    };
    assert_eq!(at(&clip, 2, 2), at(&bare, 2, 2));
    assert_ne!(at(&clip, 32, 32), at(&bare, 32, 32));

    // layers clipped to a hidden or empty base do not show at all
    let hidden = render(&cpu, find("hidden_and_empty"), BackendChoice::Cpu);
    let only_bg = {
        let mut d = find("hidden_and_empty").clone();
        d.layers.truncate(1);
        render(&cpu, &d, BackendChoice::Cpu)
    };
    assert_eq!(at(&hidden, 50, 10), at(&only_bg, 50, 10));

    // a pass through group blends its children into what is below it, an isolated group
    // blends them into each other first, so the two differ where the children overlap
    let pass = render(&cpu, find("group_pass_through"), BackendChoice::Cpu);
    let iso = render(&cpu, find("group_isolated"), BackendChoice::Cpu);
    assert_ne!(at(&pass, 32, 32), at(&iso, 32, 32));
}

fn save_gray(path: &PathBuf, buf: &GrayBuf) {
    let bytes: Vec<u8> = buf
        .data
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    image::GrayImage::from_raw(buf.rect.w, buf.rect.h, bytes)
        .expect("size")
        .save(path)
        .expect("write golden");
}

fn picture() -> ColorBuf {
    let n = 96;
    let mut data = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32, y as f32);
            let c = if (fx - 30.0).hypot(fy - 34.0) < 20.0 {
                [0.85, 0.2, 0.2]
            } else if fx > 60.0 && fy > 50.0 {
                [0.82, 0.24, 0.18]
            } else {
                [0.2, 0.3 + fy / 300.0, 0.6]
            };
            data.push(c.map(|v| (v * 65535.0) as u16));
        }
    }
    ColorBuf {
        rect: IRect::of_size(96, 96),
        data: data
            .into_iter()
            .map(|c| [c[0], c[1], c[2], 65535])
            .collect(),
    }
}

fn selections() -> Vec<(&'static str, Option<Raster>)> {
    let canvas = IRect::of_size(96, 96);
    let shape = |s: Shape, aa: bool| {
        selection::combine(None, canvas, s.bounds(), Combine::Replace, &*s.coverage(aa))
    };
    let rect = Shape::Rect {
        x: 20.3,
        y: 16.6,
        w: 40.5,
        h: 30.25,
    };
    let ellipse = Shape::Ellipse {
        x: 30.0,
        y: 30.0,
        w: 50.0,
        h: 34.0,
    };
    let star = Shape::Polygon {
        points: (0..5)
            .map(|i| {
                let a = std::f64::consts::PI * 2.0 * f64::from(i * 2) / 5.0
                    - std::f64::consts::FRAC_PI_2;
                [48.0 + 40.0 * a.cos(), 50.0 + 40.0 * a.sin()]
            })
            .collect(),
    };
    let base = shape(rect.clone(), true);
    let combo = {
        let cov = ellipse.coverage(true);
        let added =
            selection::combine(base.as_ref(), canvas, ellipse.bounds(), Combine::Add, &*cov);
        let hole = Shape::Ellipse {
            x: 40.0,
            y: 26.0,
            w: 16.0,
            h: 16.0,
        };
        let cut = selection::combine(
            added.as_ref(),
            canvas,
            hole.bounds(),
            Combine::Subtract,
            &*hole.coverage(true),
        );
        let band = Shape::Rect {
            x: 0.0,
            y: 20.0,
            w: 96.0,
            h: 36.0,
        };
        selection::combine(
            cut.as_ref(),
            canvas,
            band.bounds(),
            Combine::Intersect,
            &*band.coverage(true),
        )
    };
    let square = shape(
        Shape::Rect {
            x: 28.0,
            y: 28.0,
            w: 40.0,
            h: 40.0,
        },
        true,
    )
    .expect("square");
    let img = picture();
    vec![
        ("rect_antialiased", base.clone()),
        ("rect_hard", shape(rect, false)),
        ("ellipse", shape(ellipse, true)),
        ("star_nonzero", shape(star, true)),
        ("add_subtract_intersect", combo),
        (
            "feather_6",
            selection::feather(&square, canvas, 6.0).expect("feather"),
        ),
        (
            "expand_7",
            selection::expand(&square, canvas, 7.0).expect("expand"),
        ),
        (
            "contract_7",
            selection::expand(&square, canvas, -7.0).expect("contract"),
        ),
        (
            "border_6",
            selection::border(&square, canvas, 6.0).expect("border"),
        ),
        (
            "smooth_8",
            selection::smooth(
                &shape(
                    Shape::Polygon {
                        points: vec![
                            [10.0, 10.0],
                            [80.0, 14.0],
                            [20.0, 40.0],
                            [86.0, 80.0],
                            [12.0, 84.0],
                        ],
                    },
                    true,
                )
                .expect("poly"),
                canvas,
                8.0,
            )
            .expect("smooth"),
        ),
        ("invert", selection::invert(base.as_ref(), canvas)),
        (
            "wand_contiguous",
            selection::magic_wand(&img, (30, 34), 24.0, true, true).to_raster(canvas),
        ),
        (
            "wand_global",
            selection::magic_wand(&img, (30, 34), 24.0, false, true).to_raster(canvas),
        ),
        (
            "color_range",
            selection::color_range(&img, &[[0.85, 0.2, 0.2]], 40.0).to_raster(canvas),
        ),
        (
            "quick_select",
            selection::quick_select(&img, &[[22.0, 30.0], [36.0, 38.0]], 5.0).to_raster(canvas),
        ),
    ]
}

#[test]
fn selection_maths_matches_golden_images() {
    let out = dir("selection");
    std::fs::create_dir_all(&out).expect("golden folder");
    let canvas = IRect::of_size(96, 96);
    let mut missing = Vec::new();
    for (name, sel) in selections() {
        let buf = match &sel {
            Some(r) => GrayBuf::from_raster(r, canvas),
            None => GrayBuf::new(canvas),
        };
        let path = out.join(format!("{name}.png"));
        if update() {
            save_gray(&path, &buf);
            continue;
        }
        if !path.exists() {
            missing.push(name);
            continue;
        }
        let golden = image::open(&path)
            .expect("read golden")
            .to_luma8()
            .into_raw();
        let mine: Vec<u8> = buf
            .data
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
        let worst = golden
            .iter()
            .zip(&mine)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        // one step of eight bit rounding, for maths libraries that differ in the last bit
        assert!(
            worst <= 1,
            "{name} no longer matches {}: off by {worst}/255",
            path.display()
        );
    }
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}

/// a picture with soft edges, colour and see through parts, to show what resampling does
fn transform_source() -> ImageDocument {
    let mut doc = ImageDocument::empty("", SIZE, SIZE, Depth::U16);
    let bg = layer(&mut doc, "pattern", |x, y| {
        let check = ((x / 8.0).floor() as i32 + (y / 8.0).floor() as i32) % 2 == 0;
        let c = hsv(x / SIZE as f32, 0.8, if check { 0.95 } else { 0.45 });
        let edge = ((x - 32.0).hypot(y - 32.0) - 28.0).clamp(0.0, 1.0);
        [c[0], c[1], c[2], 1.0 - edge]
    });
    doc.active = Some(bg.id);
    doc.layers = vec![bg];
    doc
}

fn spec(corners: [[f32; 2]; 4], interpolation: Interpolation) -> TransformSpec {
    TransformSpec {
        source: [0.0, 0.0, SIZE as f32, SIZE as f32],
        corners,
        warp: None,
        source_quad: None,
        interpolation,
    }
}

fn transform_cases() -> Vec<(&'static str, TransformSpec, bool)> {
    let n = SIZE as f32;
    let turn = |deg: f32, s: f32| {
        let (sn, cs) = deg.to_radians().sin_cos();
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y): (f32, f32)| {
            let (x, y) = (x * n / 2.0 * s, y * n / 2.0 * s);
            [n / 2.0 + x * cs - y * sn, n / 2.0 + x * sn + y * cs]
        })
    };
    let wave: Vec<[f32; 2]> = (0..16)
        .map(|k| {
            let (i, j) = ((k % 4) as f32, (k / 4) as f32);
            [
                i * n / 3.0 + 6.0 * (j * 1.9).sin(),
                j * n / 3.0 + 7.0 * (i * 1.4).cos(),
            ]
        })
        .collect();
    vec![
        (
            "rotate_30_bicubic",
            spec(turn(30.0, 0.8), Interpolation::Bicubic),
            false,
        ),
        (
            "rotate_30_nearest",
            spec(turn(30.0, 0.8), Interpolation::Nearest),
            false,
        ),
        (
            "shrink_bilinear",
            spec(turn(0.0, 0.45), Interpolation::Bilinear),
            false,
        ),
        (
            "skew",
            spec(
                [[12.0, 0.0], [n, 0.0], [n - 12.0, n], [0.0, n]],
                Interpolation::Bicubic,
            ),
            false,
        ),
        (
            "perspective",
            spec(
                [[16.0, 4.0], [48.0, 4.0], [n, n - 2.0], [0.0, n - 2.0]],
                Interpolation::Bicubic,
            ),
            false,
        ),
        (
            "warp",
            TransformSpec {
                warp: Some(wave),
                ..spec(turn(0.0, 1.0), Interpolation::Bicubic)
            },
            false,
        ),
        (
            "flip_horizontal",
            spec(
                [[n, 0.0], [0.0, 0.0], [0.0, n], [n, n]],
                Interpolation::Bicubic,
            ),
            false,
        ),
        (
            "selection_moved",
            TransformSpec {
                source: [8.0, 8.0, 24.0, 24.0],
                corners: [[30.0, 26.0], [58.0, 30.0], [54.0, 58.0], [26.0, 54.0]],
                warp: None,
                source_quad: None,
                interpolation: Interpolation::Bicubic,
            },
            true,
        ),
    ]
}

fn png16(path: &PathBuf, w: u32, h: u32, data: &[u16]) {
    let img: image::ImageBuffer<image::Rgba<u16>, Vec<u16>> =
        image::ImageBuffer::from_raw(w, h, data.to_vec()).expect("size");
    img.save(path).expect("write golden");
}

/// checks `stored` against the golden png at `path`, or writes it when updating
fn check_golden(path: &PathBuf, w: u32, h: u32, stored: &[u16], missing: &mut Vec<String>) {
    if update() {
        png16(path, w, h, stored);
        return;
    }
    if !path.exists() {
        missing.push(path.display().to_string());
        return;
    }
    let golden = image::open(path).expect("read golden").to_rgba16();
    assert_eq!(
        golden.dimensions(),
        (w, h),
        "{} changed size",
        path.display()
    );
    let worst = golden
        .into_raw()
        .iter()
        .zip(stored)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 3,
        "{} no longer matches: off by {worst}/65535",
        path.display()
    );
}

fn layer_png16(r: &Raster, w: u32, h: u32) -> Vec<u16> {
    r.read(IRect::of_size(w, h))
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect()
}

#[test]
fn transforms_match_golden_images() {
    let out = dir("transform");
    std::fs::create_dir_all(&out).expect("golden folder");
    let gpu = gpu_engine();
    let mut missing = Vec::new();
    for (name, spec, lift) in transform_cases() {
        let mut doc = transform_source();
        if lift {
            let s = Shape::Ellipse {
                x: 8.0,
                y: 8.0,
                w: 24.0,
                h: 24.0,
            };
            doc.selection = selection::combine(
                None,
                doc.canvas(),
                s.bounds(),
                Combine::Replace,
                &*s.coverage(true),
            );
        }
        let before = doc.clone();
        let mut edit = Transform {
            spec: spec.clone(),
            target: TransformTarget::Layer,
            prepared: None,
        };
        edit.run(&mut doc).expect(name);
        let r = doc.layers[0].raster().expect("pixels");
        let stored = layer_png16(r, SIZE, SIZE);
        check_golden(
            &out.join(format!("{name}.png")),
            SIZE,
            SIZE,
            &stored,
            &mut missing,
        );

        // the live preview runs the same node on the graphics card and must agree
        if let Some(gpu) = &gpu {
            let plan =
                vellum_image::transform::plan(&before, TransformTarget::Layer).expect("plan");
            let preview = Preview {
                transform: Some(TransformPreview { spec, plan }),
                ..Preview::default()
            };
            let (graph, node) = build(&before, &preview);
            let req = RenderRequest::new(node, before.canvas())
                .working(before.working())
                .precision(Precision::F32);
            let on_gpu = gpu
                .render_image(&graph, &req.backend(BackendChoice::Gpu))
                .expect("gpu");
            // compared premultiplied, since a nearly clear pixel's colour means little
            let worst = on_gpu
                .data
                .iter()
                .zip(stored.as_chunks::<4>().0.iter())
                .flat_map(|(g, s)| {
                    // bicubic overshoot above full alpha is clamped when it is stored
                    let g = if g[3] > 1.0 { g.map(|v| v / g[3]) } else { *g };
                    let a = f32::from(s[3]) / 65535.0;
                    (0..4).map(move |k| {
                        let mine = if k == 3 {
                            a
                        } else {
                            f32::from(s[k]) / 65535.0 * a
                        };
                        ((g[k] - mine).abs() * 65535.0) as u32
                    })
                })
                .max()
                .unwrap_or(0);
            // nearest neighbour may pick the next pixel where a sample lands on an edge,
            // bicubic overshoot at hard edges is clamped in a slightly different order, and
            // where a perspective shrinks, f32 rounding can take one more or one fewer
            // sample per axis on one backend
            let allowed = if name.contains("nearest") {
                65535
            } else {
                1640
            };
            assert!(
                worst <= allowed,
                "{name} previews differently on the gpu: {worst}/65535"
            );
        }
    }
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}

#[test]
fn resampling_matches_golden_images() {
    let out = dir("resample");
    std::fs::create_dir_all(&out).expect("golden folder");
    let mut missing = Vec::new();
    for (interp, label) in [
        (Interpolation::Nearest, "nearest"),
        (Interpolation::Bilinear, "bilinear"),
        (Interpolation::Bicubic, "bicubic"),
    ] {
        for (w, h, size) in [(40u32, 30u32, "down"), (100, 90, "up")] {
            let mut doc = transform_source();
            ImageSize {
                width: w,
                height: h,
                interpolation: interp,
                resolution: None,
            }
            .run(&mut doc)
            .expect("image size");
            let r = doc.layers[0].raster().expect("pixels");
            let stored = layer_png16(r, w, h);
            check_golden(
                &out.join(format!("{label}_{size}.png")),
                w,
                h,
                &stored,
                &mut missing,
            );
        }
    }
    // a straightened crop and a perspective crop resample too
    for (name, crop) in [
        (
            "crop_straighten",
            Crop {
                rect: [12.0, 12.0, 40.0, 30.0],
                angle: 8.0,
                perspective: None,
                delete_outside: true,
                interpolation: Interpolation::Bicubic,
            },
        ),
        (
            "crop_perspective",
            Crop {
                rect: [0.0, 0.0, 48.0, 48.0],
                angle: 0.0,
                perspective: Some([[10.0, 6.0], [54.0, 12.0], [60.0, 58.0], [4.0, 50.0]]),
                delete_outside: true,
                interpolation: Interpolation::Bicubic,
            },
        ),
    ] {
        let mut doc = transform_source();
        let mut c = crop.clone();
        c.run(&mut doc).expect(name);
        let r = doc.layers[0].raster().expect("pixels");
        let stored = layer_png16(r, doc.width, doc.height);
        check_golden(
            &out.join(format!("{name}.png")),
            doc.width,
            doc.height,
            &stored,
            &mut missing,
        );
    }
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}
