//! render engine benchmarks, recorded in ARCHITECTURE.md section 24.
//!
//! ```text
//! cargo run --release -p vellum-gpu --example bench_render
//! ```
//!
//! 1. a 3840x2160 frame through a 10 node graph, on the gpu and the cpu: cold (nothing
//!    cached, the picture uploaded again) and warm (a slider moving on the first effect, so
//!    every effect reruns but the picture stays on the device). each frame ends as rgba8 in
//!    memory, like a viewport tile or a frame handed to an encoder.
//! 2. panning a 1920x1080 view across a 20000x20000 picture, every frame asking the
//!    viewport for what is missing, at 100 percent and at 25 percent.
//!
//! set `VELLUM_GPU=software` to allow a software rasteriser (lavapipe, warp) as the gpu.

#![allow(clippy::print_stdout, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use vellum_gpu::engine::BackendChoice;
use vellum_gpu::nodes::{self, transform::affine};
use vellum_gpu::view::{Picture, ViewSettings, ViewState, Viewport};
use vellum_gpu::{
    AlphaMode, Backend, ColorSpace, ColorTransform, Engine, EngineOptions, Graph, IRect, Node,
    NodeId, Precision, ProceduralSource, RenderRequest, SourceColor, SourcePixels, TiledImage,
};

const W: u32 = 3840;
const H: u32 = 2160;

/// a photo sized picture with some structure, as an 8 bit source like a decoded jpeg
fn photo() -> Arc<TiledImage> {
    let mut data = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let v = ((x ^ y) & 0xff) as u8;
            data.extend_from_slice(&[(x / 15) as u8, v, (y / 9) as u8, 255]);
        }
    }
    let px = SourcePixels::rgba8(IRect::of_size(W, H), AlphaMode::Straight, data);
    Arc::new(TiledImage::from_pixels(
        &px,
        SourceColor::Space(ColorSpace::SRGB),
    ))
}

/// ten nodes: picture, levels, hue/saturation, blur, sharpen, transform, gradient, blend,
/// and two colour conversions (into acescct and out to srgb)
fn ten_nodes(picture: Arc<TiledImage>, gamma: f32) -> (Graph, NodeId) {
    let mut g = Graph::new();
    let pic = g.add(Node::of(nodes::ImageNode).source(picture));
    let lv = g.add(
        Node::of(nodes::Levels)
            .param("gamma", gamma)
            .param("input_white", 240.0)
            .input(0, pic),
    );
    let hs = g.add(
        Node::of(nodes::HueSaturation)
            .param("hue", 12.0)
            .param("saturation", 15.0)
            .input(0, lv),
    );
    let bl = g.add(
        Node::of(nodes::GaussianBlur)
            .param("radius", 2.0)
            .input(0, hs),
    );
    let sh = g.add(
        Node::of(nodes::Sharpen)
            .param("amount", 80.0)
            .param("radius", 1.5)
            .input(0, bl),
    );
    let tr = g.add(
        Node::of(nodes::TransformNode)
            .param("matrix", affine(1.02, 1.5, -20.0, -10.0))
            .param("interpolation", "bilinear")
            .input(0, sh),
    );
    let gr = g.add(
        Node::of(nodes::GradientNode)
            .param("shape", "radial")
            .param("start", [W as f32 / 2.0, H as f32 / 2.0])
            .param("end", [W as f32, H as f32]),
    );
    let bd = g.add(
        Node::of(nodes::BlendNode)
            .param("mode", "soft_light")
            .param("opacity", 60.0)
            .input(0, tr)
            .input(1, gr),
    );
    // a grading style finish: into acescct, then out to srgb for the display
    let cc = g.add(
        Node::of(nodes::ColorConvertNode)
            .param("to", "acescct")
            .input(0, bd),
    );
    let out = g.add(
        Node::of(nodes::ColorConvertNode)
            .param("from", "acescct")
            .param("to", "srgb")
            .input(0, cc),
    );
    (g, out)
}

struct Timing {
    median: Duration,
    best: Duration,
}

fn time(runs: usize, mut f: impl FnMut(usize)) -> Timing {
    f(usize::MAX); // warm up: shaders compile, threads spin up
    let mut times: Vec<Duration> = (0..runs)
        .map(|i| {
            let t = Instant::now();
            f(i);
            t.elapsed()
        })
        .collect();
    times.sort();
    Timing {
        median: times[times.len() / 2],
        best: times[0],
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn bench_frame(engine: &Engine, backend: BackendChoice, label: &str) {
    let picture = photo();
    // the graph already ends in srgb
    let display = ColorTransform::identity();
    let req = |node| {
        RenderRequest::new(node, IRect::of_size(W, H))
            .precision(Precision::F16)
            .backend(backend)
    };
    let cold = time(5, |_| {
        engine.clear_caches();
        let (g, out) = ten_nodes(picture.clone(), 1.1);
        engine.render_rgba8(&g, &[req(out)], &display).unwrap();
    });
    let warm = time(10, |i| {
        // a different gamma each frame, like dragging a slider
        let (g, out) = ten_nodes(picture.clone(), 1.0 + (i % 64) as f32 * 0.01);
        engine.render_rgba8(&g, &[req(out)], &display).unwrap();
    });
    println!(
        "| {label} | {} | {} | {} |",
        ms(cold.median),
        ms(warm.median),
        ms(warm.best)
    );
}

fn bench_pan(engine: &Engine, zoom: f64, label: &str) {
    let side = 20_000u32;
    let src = Arc::new(ProceduralSource::new(side, side, |x, y| {
        let v = ((x as u32 / 64 + y as u32 / 64) % 2) as f32;
        [x / 20_000.0, 0.3 + 0.4 * v, y / 20_000.0, 1.0]
    }));
    let mut g = Graph::new();
    let pic = g.add(Node::of(nodes::ImageNode).source(src));
    let top = g.add(Node::of(nodes::Levels).param("gamma", 1.2).input(0, pic));
    let mut viewport = Viewport::new(IRect::of_size(side, side));
    let settings = ViewSettings::default();
    let picture = Picture {
        graph: &g,
        node: top,
        revision: 1,
    };
    let mut times = Vec::new();
    let mut tiles = 0usize;
    let mut view = ViewState {
        center_x: 2000.0,
        center_y: 2000.0,
        zoom,
        rotation: 0.0,
        flip_x: false,
        width: 1920,
        height: 1080,
    };
    // the first frame fills the screen, the rest pan diagonally by 40 screen pixels
    let first = Instant::now();
    let f = viewport
        .frame(engine, &picture, &view, &settings, Duration::from_secs(60))
        .unwrap();
    let fill = first.elapsed();
    let fill_tiles = f.tiles.len();
    for _ in 0..240 {
        view.center_x += 40.0 / zoom;
        view.center_y += 22.0 / zoom;
        let t = Instant::now();
        let f = viewport
            .frame(engine, &picture, &view, &settings, Duration::from_secs(60))
            .unwrap();
        times.push(t.elapsed());
        tiles += f.tiles.len();
    }
    times.sort();
    let mean = times.iter().sum::<Duration>() / times.len() as u32;
    // asking again without moving: nothing to send, only checking what the window has
    let idle = Instant::now();
    for _ in 0..100 {
        viewport
            .frame(engine, &picture, &view, &settings, Duration::from_secs(60))
            .unwrap();
    }
    let idle = idle.elapsed() / 100;
    println!(
        "| {label} | {} ({fill_tiles} tiles) | {} | {} | {:.1} | {:.2} ms |",
        ms(fill),
        ms(mean),
        ms(times[times.len() * 95 / 100]),
        tiles as f64 / times.len() as f64,
        idle.as_secs_f64() * 1000.0
    );
}

/// opening a 48 megapixel photo: copying it into tiles, then the first overview, which
/// builds every mip level from the full picture
fn bench_open(engine: &Engine, label: &str) {
    let (w, h) = (8000u32, 6000u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (i, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        px.copy_from_slice(&[(x / 32) as u8, (y / 24) as u8, ((x ^ y) & 0xff) as u8, 255]);
    }
    let px = SourcePixels::rgba8(IRect::of_size(w, h), AlphaMode::Straight, data);
    let t = Instant::now();
    let image = Arc::new(TiledImage::from_pixels(
        &px,
        SourceColor::Space(ColorSpace::SRGB),
    ));
    let tiled = t.elapsed();
    let mut g = Graph::new();
    let node = g.add(Node::of(nodes::ImageNode).source(image));
    let mut viewport = Viewport::new(IRect::of_size(w, h));
    let view = ViewState {
        center_x: f64::from(w) / 2.0,
        center_y: f64::from(h) / 2.0,
        zoom: 0.2,
        rotation: 0.0,
        flip_x: false,
        width: 1920,
        height: 1080,
    };
    let picture = Picture {
        graph: &g,
        node,
        revision: 1,
    };
    let t = Instant::now();
    viewport
        .frame(
            engine,
            &picture,
            &view,
            &ViewSettings::default(),
            Duration::from_secs(60),
        )
        .unwrap();
    println!("| {label} | {} | {} |", ms(tiled), ms(t.elapsed()));
}

fn main() {
    let cpu = Engine::cpu_only();
    let gpu = Engine::new(EngineOptions::default());
    let status = gpu.status();
    let has_gpu = status.backend == Backend::Gpu;
    println!(
        "gpu: {}{}",
        status.adapter.as_deref().unwrap_or("none"),
        if status.software { " (software)" } else { "" }
    );
    println!("cpu threads: {}", rayon::current_num_threads());
    println!();
    println!("4k frame, 10 nodes, f16, ending as rgba8 in memory");
    println!();
    println!("| backend | cold | warm, median | warm, best |");
    println!("|---|---|---|---|");
    if has_gpu {
        bench_frame(&gpu, BackendChoice::Gpu, "gpu");
    }
    bench_frame(&cpu, BackendChoice::Cpu, "cpu");
    println!();
    println!("opening an 8000x6000 photo");
    println!();
    println!("| backend | into tiles | first screen, fitted |");
    println!("|---|---|---|");
    if has_gpu {
        bench_open(&gpu, "gpu");
    }
    bench_open(&cpu, "cpu");
    println!();
    println!("panning a 1920x1080 view over 20000x20000, 240 frames");
    println!();
    println!(
        "| run | first screen | per frame, mean | per frame, p95 | new tiles per frame | still frame |"
    );
    println!("|---|---|---|---|---|---|");
    if has_gpu {
        bench_pan(&gpu, 1.0, "gpu, 100%");
        bench_pan(&gpu, 0.25, "gpu, 25%");
    }
    bench_pan(&cpu, 1.0, "cpu, 100%");
    bench_pan(&cpu, 0.25, "cpu, 25%");
}
