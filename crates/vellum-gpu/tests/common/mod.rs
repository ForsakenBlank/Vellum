#![allow(dead_code)]

use std::sync::Arc;

use vellum_gpu::gpu::GpuOptions;
use vellum_gpu::{Backend, CpuImage, Engine, EngineOptions, IRect, ImageSource, ProceduralSource};

/// an engine on a gpu, software rasterisers allowed so ci runs the gpu path with lavapipe
/// or warp. none (and the test is skipped) when there is no gpu, unless
/// `VELLUM_REQUIRE_GPU` is set, which turns the skip into a failure.
pub fn gpu_engine() -> Option<Engine> {
    let engine = Engine::new(EngineOptions {
        gpu: GpuOptions::allowing_software(),
        ..Default::default()
    });
    if engine.status().backend == Backend::Gpu {
        return Some(engine);
    }
    // ci sets this to an empty string where it does not apply
    if std::env::var("VELLUM_REQUIRE_GPU").is_ok_and(|v| !v.is_empty()) {
        panic!(
            "no gpu adapter, and VELLUM_REQUIRE_GPU is set: {:?}",
            engine.status().note
        );
    }
    eprintln!("skipping gpu checks: {:?}", engine.status().note);
    None
}

/// a colourful test picture with soft transparency towards the bottom right
pub fn test_picture(w: u32, h: u32) -> Arc<dyn ImageSource> {
    Arc::new(ProceduralSource::new(w, h, move |x, y| {
        let u = x / w as f32;
        let v = y / h as f32;
        let ring = ((x - w as f32 * 0.5).hypot(y - h as f32 * 0.5) * 0.15).sin() * 0.5 + 0.5;
        [u, v, ring, (1.2 - 0.6 * (u + v)).clamp(0.0, 1.0)]
    }))
}

pub fn max_diff(a: &CpuImage, b: &CpuImage) -> (f32, i32, i32) {
    assert_eq!(a.rect, b.rect, "different rects");
    let mut worst = (0.0f32, 0, 0);
    for y in a.rect.y..a.rect.bottom() {
        for x in a.rect.x..a.rect.right() {
            let (p, q) = (a.get(x, y), b.get(x, y));
            for k in 0..4 {
                let d = (p[k] - q[k]).abs();
                if d > worst.0 || d.is_nan() {
                    worst = (if d.is_nan() { f32::INFINITY } else { d }, x, y);
                }
            }
        }
    }
    worst
}

pub fn assert_close(label: &str, a: &CpuImage, b: &CpuImage, tol: f32) {
    let (d, x, y) = max_diff(a, b);
    assert!(
        d <= tol,
        "{label}: differs by {d} at ({x}, {y}): {:?} vs {:?}",
        a.get(x, y),
        b.get(x, y)
    );
}

pub fn region(x: i32, y: i32, w: u32, h: u32) -> IRect {
    IRect::new(x, y, w, h)
}
