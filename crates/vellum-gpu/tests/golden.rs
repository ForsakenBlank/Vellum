//! golden images for every blend mode, in `tests/golden/blend/` at the repo root.
//!
//! each image is a colourful backdrop with a fading bottom edge, blended with a layer of
//! other colours that fades out on the right, in an srgb working space like photoshop's 8
//! and 16 bit documents. the cpu reference must match the stored 16 bit png and the gpu
//! must match the cpu. run with `VELLUM_UPDATE_GOLDEN=1` to write new references after a
//! deliberate change, then look at them before committing.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{assert_close, gpu_engine};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::nodes::{self, blend::mode_param};
use vellum_gpu::{
    BackendChoice, BlendMode, CpuImage, Engine, Graph, IRect, Node, NodeId, Precision,
    ProceduralSource, WorkingSpace,
};

const SIZE: u32 = 64;

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

fn scene(mode: BlendMode) -> (Graph, NodeId) {
    let n = SIZE as f32;
    let backdrop = Arc::new(ProceduralSource::new(SIZE, SIZE, move |x, y| {
        let c = hsv(x / n, 0.85, 0.15 + 0.85 * (1.0 - y / n));
        let a = if y < n * 0.75 {
            1.0
        } else {
            1.0 - (y - n * 0.75) / (n * 0.25) * 0.75
        };
        [c[0], c[1], c[2], a]
    }));
    let layer = Arc::new(ProceduralSource::new(SIZE, SIZE, move |x, y| {
        let c = hsv(
            0.5 + y / n * 0.6,
            (x / n).clamp(0.0, 1.0),
            0.2 + 0.8 * (y / n),
        );
        let a = if x < n * 0.75 {
            1.0
        } else {
            1.0 - (x - n * 0.75) / (n * 0.25) * 0.8
        };
        [c[0], c[1], c[2], a]
    }));
    let mut g = Graph::new();
    let b = g.add(Node::of(nodes::ImageNode).source(backdrop));
    let l = g.add(Node::of(nodes::ImageNode).source(layer));
    let top = g.add(
        Node::of(nodes::BlendNode)
            .param("mode", mode_param(mode))
            .input(0, b)
            .input(1, l),
    );
    (g, top)
}

fn request(top: NodeId) -> RenderRequest {
    RenderRequest::new(top, IRect::of_size(SIZE, SIZE))
        .working(WorkingSpace::SRGB)
        .precision(Precision::F32)
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/blend")
}

/// straight 16 bit rgba, which is what gets stored
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

fn from_png16(data: &[u16]) -> CpuImage {
    let px = data
        .chunks_exact(4)
        .map(|c| {
            let s = [0, 1, 2, 3].map(|k| f32::from(c[k]) / 65535.0);
            [s[0] * s[3], s[1] * s[3], s[2] * s[3], s[3]]
        })
        .collect();
    CpuImage {
        rect: IRect::of_size(SIZE, SIZE),
        data: px,
    }
}

fn save(path: &PathBuf, data: Vec<u16>, w: u32, h: u32) {
    let img: image::ImageBuffer<image::Rgba<u16>, Vec<u16>> =
        image::ImageBuffer::from_raw(w, h, data).expect("buffer size");
    img.save(path).expect("write golden image");
}

#[test]
fn blend_modes_match_their_golden_images() {
    let update = std::env::var_os("VELLUM_UPDATE_GOLDEN").is_some();
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("golden folder");
    let cpu = Engine::cpu_only();
    let gpu = gpu_engine();
    let mut sheet = vec![0u16; (SIZE * 7 * SIZE * 4 * 4) as usize];
    let mut missing = Vec::new();

    for (i, mode) in BlendMode::ALL.into_iter().enumerate() {
        let (graph, top) = scene(mode);
        let req = request(top);
        let reference = cpu
            .render_image(&graph, &req.backend(BackendChoice::Cpu))
            .expect("cpu");
        let path = dir.join(format!("{}.png", mode.slug()));
        let stored = to_png16(&reference);

        // a contact sheet of every mode, seven to a row, for looking at by eye
        let (cx, cy) = ((i % 7) as u32 * SIZE, (i / 7) as u32 * SIZE);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let src = ((y * SIZE + x) * 4) as usize;
                let dst = (((cy + y) * SIZE * 7 + cx + x) * 4) as usize;
                sheet[dst..dst + 4].copy_from_slice(&stored[src..src + 4]);
            }
        }

        if update {
            save(&path, stored, SIZE, SIZE);
        } else if !path.exists() {
            missing.push(mode.slug());
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
                worst <= 2,
                "{mode:?} no longer matches {}: off by {worst}/65535",
                path.display()
            );
        }

        if let Some(gpu) = &gpu {
            let on_gpu = gpu
                .render_image(&graph, &req.backend(BackendChoice::Gpu))
                .expect("gpu");
            assert_close(&format!("{mode:?} on the gpu"), &reference, &on_gpu, 1.5e-3);
            let decoded = from_png16(&to_png16(&on_gpu));
            assert_close(
                &format!("{mode:?} gpu vs golden"),
                &from_png16(&to_png16(&reference)),
                &decoded,
                2e-3,
            );
        }
    }
    if update {
        save(&dir.join("all.png"), sheet, SIZE * 7, SIZE * 4);
    }
    assert!(
        missing.is_empty(),
        "no golden images for {missing:?}; run with VELLUM_UPDATE_GOLDEN=1"
    );
}

#[test]
fn modes_also_agree_in_a_linear_working_space() {
    let Some(gpu) = gpu_engine() else { return };
    let cpu = Engine::cpu_only();
    for mode in BlendMode::ALL {
        let (graph, top) = scene(mode);
        let req = request(top).working(WorkingSpace::LINEAR_SRGB);
        let a = cpu
            .render_image(&graph, &req.backend(BackendChoice::Cpu))
            .expect("cpu");
        let b = gpu
            .render_image(&graph, &req.backend(BackendChoice::Gpu))
            .expect("gpu");
        assert_close(&format!("{mode:?} linear"), &a, &b, 1.5e-3);
    }
}

#[test]
fn normal_mode_over_opaque_is_the_layer_wherever_the_layer_is_opaque() {
    let cpu = Engine::cpu_only();
    let (graph, top) = scene(BlendMode::Normal);
    let out = cpu.render_image(&graph, &request(top)).expect("render");
    let (lg, ltop) = {
        let (g, t) = scene(BlendMode::Normal);
        let layer = g.node(t).expect("blend").inputs[1].expect("layer");
        (g, layer)
    };
    let _ = ltop;
    let layer = cpu.render_image(&lg, &request(ltop)).expect("layer");
    for y in 0..SIZE as i32 {
        for x in 0..(SIZE as i32 * 3 / 4) {
            let (o, l) = (out.get(x, y), layer.get(x, y));
            for k in 0..4 {
                assert!((o[k] - l[k]).abs() < 1e-6, "({x}, {y})");
            }
        }
    }
}
