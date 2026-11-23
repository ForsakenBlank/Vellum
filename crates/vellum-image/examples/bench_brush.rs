//! brush latency on a 4k canvas: how long from pointer samples arriving to the changed
//! display tiles being ready to send to the window.
//!
//! `cargo run --release -p vellum-image --example bench_brush`
//!
//! a 3840x2160 document with a photo-like background and two half covered layers under the
//! layer being painted. a 60 pixel brush is dragged across the canvas in 240 batches of
//! four pointer samples, about what a pen sends per frame at 240 Hz. each batch paints its
//! dabs, rebuilds the graph with the stroke as a preview, and renders every display tile
//! that changed for a 1920x1080 view, once fitted to the screen and once at 100 percent
//! centred on the canvas.

// a benchmark reports on stdout
#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

use vellum_gpu::gpu::GpuOptions;
use vellum_gpu::view::{Picture, ViewSettings, ViewState, Viewport};
use vellum_gpu::{Backend, BackendChoice, Engine, EngineOptions, IRect};
use vellum_image::compose::{Preview, build};
use vellum_image::document::{ImageDocument, Layer};
use vellum_image::paint::{BrushSettings, Stroke, StrokePoint};
use vellum_image::raster::{Depth, Raster};

const W: u32 = 3840;
const H: u32 = 2160;

fn document() -> ImageDocument {
    let mut doc = ImageDocument::empty("bench", W, H, Depth::U8);
    let mut bg = Raster::rgba(Depth::U8);
    bg.update(IRect::of_size(W, H), |x, y, px| {
        let (u, v) = (x as f32 / W as f32, y as f32 / H as f32);
        let ring = ((x as f32 - 1900.0).hypot(y as f32 - 1000.0) * 0.02).sin() * 0.5 + 0.5;
        px.copy_from_slice(&[u, v * 0.8 + 0.1, ring * 0.6 + 0.2, 1.0]);
    });
    let mut half = Raster::rgba(Depth::U8);
    half.update(IRect::new(0, 0, W / 2, H), |_, y, px| {
        px.copy_from_slice(&[0.9, 0.3, 0.2, (y % 512) as f32 / 512.0]);
    });
    let mut stripes = Raster::rgba(Depth::U8);
    stripes.update(IRect::new(0, H as i32 / 2, W, H / 2), |x, _, px| {
        let a = if (x / 40) % 2 == 0 { 0.6 } else { 0.0 };
        px.copy_from_slice(&[0.1, 0.4, 0.9, a]);
    });
    let ids: Vec<_> = (0..4).map(|_| doc.new_id()).collect();
    doc.layers.push(Layer::pixels(ids[0], "Background", bg));
    let mut l1 = Layer::pixels(ids[1], "Half", half);
    l1.blend = vellum_gpu::BlendMode::Multiply;
    doc.layers.push(l1);
    let mut l2 = Layer::pixels(ids[2], "Stripes", stripes);
    l2.opacity = 0.7;
    doc.layers.push(l2);
    doc.layers
        .push(Layer::pixels(ids[3], "Paint", Raster::rgba(Depth::U8)));
    doc.active = Some(ids[3]);
    doc
}

fn path(i: usize) -> StrokePoint {
    // a wide s curve across the canvas, about 6 pixels between samples
    let t = i as f32 / 960.0;
    StrokePoint {
        x: 200.0 + t * 3400.0,
        y: 1080.0 + (t * std::f32::consts::TAU * 1.5).sin() * 700.0,
        pressure: 0.6 + 0.4 * (t * 9.0).sin().abs(),
        tilt_x: 0.0,
        tilt_y: 0.0,
    }
}

struct Times {
    paint: Vec<Duration>,
    graph: Vec<Duration>,
    tiles: Vec<Duration>,
    total: Vec<Duration>,
    sent: usize,
}

fn stats(label: &str, v: &mut [Duration]) {
    v.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let n = v.len().max(1);
    println!(
        "  {label:<8} median {:>6.2} ms   p95 {:>6.2} ms   max {:>6.2} ms",
        ms(v[n / 2]),
        ms(v[(n * 95 / 100).min(n - 1)]),
        ms(v[n - 1])
    );
}

fn run(engine: &Engine, zoom: f64, backend: BackendChoice) -> Times {
    let doc = document();
    let settings = ViewSettings {
        working: doc.working(),
        precision: doc.precision(),
        backend,
        ..Default::default()
    };
    // the view holds still while painting; at 100 percent part of the stroke runs off
    // screen and only costs its painting
    let view = ViewState {
        center_x: f64::from(W) / 2.0,
        center_y: f64::from(H) / 2.0,
        zoom,
        rotation: 0.0,
        flip_x: false,
        width: 1920,
        height: 1080,
    };
    let mut viewport = Viewport::new(doc.canvas());
    let (g, n) = build(&doc, &Preview::default());
    // the first screen is not part of the stroke
    viewport
        .frame(
            engine,
            &Picture {
                graph: &g,
                node: n,
                revision: 1,
            },
            &view,
            &settings,
            Duration::from_secs(60),
        )
        .expect("first frame");
    let mut stroke = Stroke::new(
        &doc,
        BrushSettings {
            size: 60.0,
            hardness: 0.8,
            spacing: 0.1,
            smoothing: 0.1,
            color: [0.1, 0.1, 0.1],
            ..Default::default()
        },
    )
    .expect("stroke");
    let mut t = Times {
        paint: vec![],
        graph: vec![],
        tiles: vec![],
        total: vec![],
        sent: 0,
    };
    for (batch, revision) in (0..240).zip(2..) {
        let pts: Vec<StrokePoint> = (0..4).map(|k| path(batch * 4 + k)).collect();
        let start = Instant::now();
        stroke.add(&pts);
        let painted = Instant::now();
        let preview = Preview {
            replace: Some((stroke.part.clone(), stroke.result().clone())),
            ..Default::default()
        };
        let (g, n) = build(&doc, &preview);
        let built = Instant::now();
        let mut sent = 0;
        loop {
            let update = viewport
                .frame(
                    engine,
                    &Picture {
                        graph: &g,
                        node: n,
                        revision,
                    },
                    &view,
                    &settings,
                    Duration::from_secs(60),
                )
                .expect("frame");
            sent += update.tiles.len();
            if update.pending == 0 {
                break;
            }
        }
        let done = Instant::now();
        t.paint.push(painted - start);
        t.graph.push(built - painted);
        t.tiles.push(done - built);
        t.total.push(done - start);
        t.sent += sent;
    }
    t
}

fn report(label: &str, engine: &Engine, backend: BackendChoice) {
    for (name, zoom) in [("fit (about 50%)", 0.5), ("100%", 1.0)] {
        let mut t = run(engine, zoom, backend);
        println!("{label}, {name}: {} tiles sent over 240 batches", t.sent);
        stats("paint", &mut t.paint);
        stats("graph", &mut t.graph);
        stats("tiles", &mut t.tiles);
        stats("total", &mut t.total);
    }
}

fn main() {
    println!("4k brush latency, 240 batches of 4 samples, 60 px brush, 1920x1080 view");
    let cpu = Engine::cpu_only();
    report("cpu", &cpu, BackendChoice::Cpu);
    let gpu = Engine::new(EngineOptions {
        gpu: GpuOptions::allowing_software(),
        ..Default::default()
    });
    if gpu.status().backend == Backend::Gpu {
        let name = gpu.status().adapter.unwrap_or_default();
        report(&format!("gpu ({name})"), &gpu, BackendChoice::Gpu);
    } else {
        println!("no gpu adapter, gpu rows skipped");
    }
}
