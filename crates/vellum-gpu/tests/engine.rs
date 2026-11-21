//! the engine end to end: every node on the gpu against its cpu twin, caching and
//! invalidation, recovery from a lost device, and images bigger than a gpu texture.

mod common;

use std::sync::Arc;

use common::{assert_close, gpu_engine, region, test_picture};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::nodes::{self, blend::mode_param, transform::affine};
use vellum_gpu::params::GradientStop;
use vellum_gpu::{
    BackendChoice, BlendMode, ColorSpace, ColorTransform, Engine, Graph, IRect, IccProfile, Node,
    NodeId, Precision, ProceduralSource, RenderError, SourceColor, TiledImage, WorkingSpace,
    render_tiled,
};

/// a scene that runs one node of each kind, so a failure names the node that broke
fn scenes() -> Vec<(&'static str, Graph, NodeId)> {
    let mut out = Vec::new();
    let mut add = |name: &'static str, build: &dyn Fn(&mut Graph, NodeId) -> NodeId| {
        let mut g = Graph::new();
        let pic = g.add(Node::of(nodes::ImageNode).source(test_picture(160, 120)));
        let top = build(&mut g, pic);
        out.push((name, g, top));
    };
    add("image", &|_, pic| pic);
    add("solid", &|g, _| {
        g.add(Node::of(nodes::SolidNode).param("color", [0.9, 0.4, 0.1, 0.8]))
    });
    add("gradient linear", &|g, _| gradient(g, "linear"));
    add("gradient radial", &|g, _| gradient(g, "radial"));
    add("gradient angle", &|g, _| gradient(g, "angle"));
    add("gradient reflected", &|g, _| gradient(g, "reflected"));
    add("gradient diamond", &|g, _| gradient(g, "diamond"));
    add("blend with mask", &|g, pic| {
        let back = gradient(g, "radial");
        let mask = gradient(g, "linear");
        g.add(
            Node::of(nodes::BlendNode)
                .param("mode", "overlay")
                .param("opacity", 80.0)
                .input(0, back)
                .input(1, pic)
                .input(2, mask),
        )
    });
    add("mask", &|g, pic| {
        let mask = gradient(g, "diamond");
        g.add(
            Node::of(nodes::MaskNode)
                .param("density", 75.0)
                .input(0, pic)
                .input(1, mask),
        )
    });
    add("transform bicubic", &|g, pic| {
        transform(g, pic, "bicubic", affine(0.7, 30.0, 40.0, -10.0))
    });
    add("transform bilinear shrink", &|g, pic| {
        transform(g, pic, "bilinear", affine(0.3, -12.0, 20.0, 15.0))
    });
    add("transform nearest", &|g, pic| {
        transform(g, pic, "nearest", affine(1.5, 45.0, 30.0, 0.0))
    });
    add("transform perspective", &|g, pic| {
        transform(
            g,
            pic,
            "bilinear",
            [1.0, 0.1, 5.0, 0.05, 0.9, 3.0, 0.0015, 0.0005, 1.0],
        )
    });
    add("gaussian blur", &|g, pic| {
        g.add(
            Node::of(nodes::GaussianBlur)
                .param("radius", 3.0)
                .input(0, pic),
        )
    });
    add("gaussian blur fading edges", &|g, pic| {
        g.add(
            Node::of(nodes::GaussianBlur)
                .param("radius", 5.0)
                .param("edges", "transparent")
                .input(0, pic),
        )
    });
    add("levels", &|g, pic| {
        g.add(
            Node::of(nodes::Levels)
                .param("input_black", 20.0)
                .param("input_white", 230.0)
                .param("gamma", 1.4)
                .param("output_black", 10.0)
                .input(0, pic),
        )
    });
    add("levels one channel", &|g, pic| {
        g.add(
            Node::of(nodes::Levels)
                .param("channel", "green")
                .param("gamma", 0.6)
                .input(0, pic),
        )
    });
    add("hue saturation", &|g, pic| {
        g.add(
            Node::of(nodes::HueSaturation)
                .param("hue", 70.0)
                .param("saturation", 40.0)
                .param("lightness", -20.0)
                .input(0, pic),
        )
    });
    add("colorize", &|g, pic| {
        g.add(
            Node::of(nodes::HueSaturation)
                .param("colorize", true)
                .param("saturation", 50.0)
                .input(0, pic),
        )
    });
    add("sharpen", &|g, pic| {
        g.add(
            Node::of(nodes::Sharpen)
                .param("amount", 150.0)
                .param("radius", 2.0)
                .input(0, pic),
        )
    });
    add("convert colour space", &|g, pic| {
        g.add(
            Node::of(nodes::ColorConvertNode)
                .param("to", "acescct")
                .input(0, pic),
        )
    });
    add("icc lut", &|g, pic| {
        let t = IccProfile::display_p3()
            .from_working(WorkingSpace::LINEAR_SRGB)
            .expect("p3 profile");
        g.add(Node::of(nodes::LutNode).transform(t).input(0, pic))
    });
    add("output over a background", &|g, pic| {
        g.add(
            Node::of(nodes::OutputNode)
                .param("background", [0.2, 0.3, 0.4, 1.0])
                .input(0, pic),
        )
    });
    out
}

fn gradient(g: &mut Graph, shape: &str) -> NodeId {
    g.add(
        Node::of(nodes::GradientNode)
            .param("shape", shape)
            .param("start", [80.0, 60.0])
            .param("end", [150.0, 20.0])
            .param(
                "stops",
                vec![
                    GradientStop {
                        offset: 0.0,
                        color: [1.0, 0.1, 0.1, 1.0],
                    },
                    GradientStop {
                        offset: 0.4,
                        color: [0.1, 1.0, 0.2, 0.5],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [0.1, 0.2, 1.0, 1.0],
                    },
                ],
            ),
    )
}

fn transform(g: &mut Graph, input: NodeId, interpolation: &str, m: [f32; 9]) -> NodeId {
    g.add(
        Node::of(nodes::TransformNode)
            .param("matrix", m)
            .param("interpolation", interpolation)
            .input(0, input),
    )
}

#[test]
fn every_node_agrees_between_cpu_and_gpu() {
    let Some(gpu) = gpu_engine() else { return };
    let cpu = Engine::cpu_only();
    for working in [WorkingSpace::LINEAR_SRGB, WorkingSpace::SRGB] {
        for level in [0, 1] {
            for (name, graph, top) in scenes() {
                let req = RenderRequest::new(top, region(-8, -6, 176, 132))
                    .level(level)
                    .working(working)
                    .precision(Precision::F32);
                let a = cpu
                    .render_image(&graph, &req.backend(BackendChoice::Cpu))
                    .expect(name);
                let b = gpu
                    .render_image(&graph, &req.backend(BackendChoice::Gpu))
                    .expect(name);
                let label = format!("{name} ({working:?}, level {level})");
                if name == "transform nearest" {
                    // a sample exactly on a pixel edge may round either way on the two
                    // sides; allow a few such pixels, but no other differences
                    let off = a
                        .data
                        .iter()
                        .zip(&b.data)
                        .filter(|(p, q)| (0..4).any(|k| (p[k] - q[k]).abs() > 2e-3))
                        .count();
                    assert!(off * 200 <= a.data.len(), "{label}: {off} pixels differ");
                } else {
                    assert_close(&label, &a, &b, 2e-3);
                }
            }
        }
    }
}

#[test]
fn half_and_eight_bit_pipelines_stay_close_to_full_precision() {
    let Some(gpu) = gpu_engine() else { return };
    let (_, graph, top) = scenes()
        .into_iter()
        .find(|s| s.0 == "blend with mask")
        .expect("scene");
    let req = RenderRequest::new(top, region(0, 0, 160, 120)).backend(BackendChoice::Gpu);
    let full = gpu
        .render_image(&graph, &req.precision(Precision::F32))
        .expect("f32");
    let half = gpu
        .render_image(&graph, &req.precision(Precision::F16))
        .expect("f16");
    let byte = gpu
        .render_image(&graph, &req.precision(Precision::U8))
        .expect("u8");
    assert_close("f16", &full, &half, 4e-3);
    assert_close("u8", &full, &byte, 1.5 / 255.0 * 3.0);
}

#[test]
fn identical_renders_come_from_the_cache() {
    let engine = gpu_engine().unwrap_or_else(Engine::cpu_only);
    let mut g = Graph::new();
    let pic = g.add(Node::of(nodes::ImageNode).source(test_picture(200, 200)));
    let blur = g.add(
        Node::of(nodes::GaussianBlur)
            .param("radius", 2.0)
            .input(0, pic),
    );
    let levels = g.add(Node::of(nodes::Levels).param("gamma", 1.2).input(0, blur));
    let req = RenderRequest::new(levels, region(0, 0, 128, 128));

    engine.render_image(&g, &req).expect("first");
    assert_eq!(engine.stats().nodes_run, 3);
    engine.render_image(&g, &req).expect("second");
    assert_eq!(engine.stats().nodes_run, 0);
    assert_eq!(engine.stats().cache_hits, 1);

    // changing the last node only reruns that node
    g.node_mut(levels)
        .expect("node")
        .params
        .insert("gamma".into(), 0.8.into());
    engine.render_image(&g, &req).expect("third");
    assert_eq!(engine.stats().nodes_run, 1);

    // a rebuilt graph with the same content is still cached
    let mut again = Graph::new();
    let pic2 = again.add(g.node(pic).expect("pic").clone());
    let blur2 = again.add(
        Node::of(nodes::GaussianBlur)
            .param("radius", 2.0)
            .input(0, pic2),
    );
    let levels2 = again.add(Node::of(nodes::Levels).param("gamma", 0.8).input(0, blur2));
    engine
        .render_image(&again, &RenderRequest::new(levels2, req.region))
        .expect("rebuilt");
    assert_eq!(engine.stats().nodes_run, 0);
}

#[test]
fn writing_pixels_only_rerenders_what_they_touch() {
    let engine = Engine::cpu_only();
    let img = Arc::new(TiledImage::new(
        512,
        256,
        vellum_gpu::SourceFormat::Rgba8,
        SourceColor::Space(ColorSpace::SRGB),
    ));
    let mut g = Graph::new();
    let src = g.add(Node::of(nodes::ImageNode).source(img.clone()));
    let left = RenderRequest::new(src, region(0, 0, 256, 256));
    let right = RenderRequest::new(src, region(256, 0, 256, 256));
    let k_left = engine.key(&g, &left).expect("key");
    let k_right = engine.key(&g, &right).expect("key");
    img.write(&vellum_gpu::SourcePixels::rgba8(
        IRect::new(300, 20, 2, 1),
        vellum_gpu::AlphaMode::Straight,
        vec![255, 0, 0, 255, 255, 0, 0, 255],
    ));
    assert_eq!(engine.key(&g, &left).expect("key"), k_left);
    assert_ne!(engine.key(&g, &right).expect("key"), k_right);
    let out = engine.render_image(&g, &right).expect("render");
    assert!((out.get(300, 20)[0] - 1.0).abs() < 1e-6 && out.get(299, 20)[3] == 0.0);
}

#[test]
fn bypassed_and_empty_nodes_cost_nothing() {
    let engine = Engine::cpu_only();
    let mut g = Graph::new();
    let pic = g.add(Node::of(nodes::ImageNode).source(test_picture(64, 64)));
    let mut blur = Node::of(nodes::GaussianBlur)
        .param("radius", 30.0)
        .input(0, pic);
    blur.bypass = true;
    let blur = g.add(blur);
    let out = engine
        .render_image(&g, &RenderRequest::new(blur, region(0, 0, 64, 64)))
        .expect("render");
    let direct = engine
        .render_image(&g, &RenderRequest::new(pic, region(0, 0, 64, 64)))
        .expect("render");
    assert_close("bypass", &out, &direct, 0.0);
    // nothing outside the picture's bounds is ever computed
    let far = engine
        .render_image(&g, &RenderRequest::new(pic, region(1000, 1000, 32, 32)))
        .expect("render");
    assert!(far.data.iter().all(|p| p[3] == 0.0));
    assert_eq!(engine.stats().nodes_run, 0);
}

#[test]
fn a_lost_device_is_reopened_and_rendering_carries_on() {
    let Some(engine) = gpu_engine() else { return };
    let (_, graph, top) = scenes()
        .into_iter()
        .find(|s| s.0 == "sharpen")
        .expect("scene");
    let req = RenderRequest::new(top, region(0, 0, 96, 96)).precision(Precision::F32);
    let before = engine.render_image(&graph, &req).expect("before");
    engine.gpu_context().expect("gpu").simulate_loss();
    let after = engine.render_image(&graph, &req).expect("after the loss");
    assert_close("after recovery", &before, &after, 2e-3);
    let status = engine.status();
    assert_eq!(status.recoveries, 1);
    assert_eq!(
        status.backend,
        vellum_gpu::Backend::Gpu,
        "{:?}",
        status.note
    );
    // and the new device really works
    let again = engine
        .render_image(&graph, &req.backend(BackendChoice::Gpu))
        .expect("gpu again");
    assert_close("new device", &before, &again, 2e-3);
}

#[test]
fn requests_wider_than_a_gpu_texture_fall_back_to_the_cpu() {
    let Some(engine) = gpu_engine() else { return };
    let max = engine.gpu_context().expect("gpu").max_texture;
    let mut g = Graph::new();
    let pic = g.add(Node::of(nodes::ImageNode).source(test_picture(max + 600, 64)));
    let req = RenderRequest::new(pic, region(0, 0, max + 600, 4));
    let wide = engine.render_image(&g, &req).expect("auto falls back");
    assert_eq!(wide.rect.w, max + 600);
    assert!(!engine.stats().gpu);
    let err = engine
        .render_image(&g, &req.backend(BackendChoice::Gpu))
        .err();
    assert!(
        matches!(
            err,
            Some(RenderError::Gpu(vellum_gpu::GpuError::TooLarge(_)))
        ),
        "{err:?}"
    );
}

/// 30000 x 30000 is far beyond any gpu texture. tiles must stitch without seams and memory
/// must stay inside the budget.
fn huge_graph() -> (Graph, NodeId) {
    let mut g = Graph::new();
    let src = Arc::new(ProceduralSource::new(30_000, 30_000, |x, y| {
        let v = ((x * 0.013).sin() * (y * 0.017).cos()) * 0.5 + 0.5;
        [v, (x / 30_000.0), (y / 30_000.0), 1.0]
    }));
    let pic = g.add(Node::of(nodes::ImageNode).source(src));
    let blur = g.add(
        Node::of(nodes::GaussianBlur)
            .param("radius", 3.0)
            .input(0, pic),
    );
    let top = g.add(Node::of(nodes::Levels).param("gamma", 1.3).input(0, blur));
    (g, top)
}

fn stitched(engine: &Engine, g: &Graph, req: &RenderRequest, tile: u32) -> vellum_gpu::CpuImage {
    let mut out = vellum_gpu::CpuImage::transparent(req.region);
    render_tiled(engine, g, req, tile, |_, img| {
        out.paste(&img);
        Ok(())
    })
    .expect("tiled render");
    out
}

#[test]
fn huge_images_render_in_tiles_without_seams() {
    let (g, top) = huge_graph();
    let engines: Vec<Engine> = std::iter::once(Engine::cpu_only())
        .chain(gpu_engine())
        .collect();
    for engine in engines {
        engine.set_budgets(64 << 20, 64 << 20);
        for area in [
            region(14_900, 14_950, 300, 200),
            region(29_800, 29_870, 300, 200),
        ] {
            let req = RenderRequest::new(top, area).precision(Precision::F32);
            let whole = engine.render_image(&g, &req).expect("whole");
            engine.clear_caches();
            let tiles = stitched(&engine, &g, &req, 64);
            assert_close("seams", &whole, &tiles, 1e-5);
            // the far edge of the picture is transparent past 30000
            if area.right() > 30_000 {
                assert_eq!(whole.get(30_050, 29_900)[3], 0.0);
            }
        }
        let status = engine.status();
        assert!(status.gpu_cache_bytes <= 64 << 20 && status.cpu_cache_bytes <= 64 << 20);
    }
}

#[test]
fn a_whole_huge_image_renders_at_a_small_level() {
    let (g, top) = huge_graph();
    let engine = gpu_engine().unwrap_or_else(Engine::cpu_only);
    engine.set_budgets(128 << 20, 128 << 20);
    // level 5 is 1/32 size: 938 x 938
    let req = RenderRequest::new(top, IRect::of_size(30_000, 30_000).at_level(5)).level(5);
    let mut tiles = 0;
    render_tiled(&engine, &g, &req, 512, |rect, img| {
        assert_eq!(img.rect, rect);
        tiles += 1;
        Ok(())
    })
    .expect("render");
    assert_eq!(tiles, 4);
    let st = engine.status();
    assert!(st.gpu_cache_bytes <= 128 << 20 && st.cpu_cache_bytes <= 128 << 20);
}

#[test]
fn cancelling_a_tiled_render_stops_it() {
    let (g, top) = huge_graph();
    let engine = Engine::cpu_only();
    let mut seen = 0;
    let result = render_tiled(
        &engine,
        &g,
        &RenderRequest::new(top, region(0, 0, 1024, 1024)),
        256,
        |_, _| {
            seen += 1;
            if seen == 2 {
                Err(RenderError::Cancelled)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(RenderError::Cancelled)));
    assert_eq!(seen, 2);
}

#[test]
fn display_tiles_match_between_backends() {
    let Some(gpu) = gpu_engine() else { return };
    let cpu = Engine::cpu_only();
    let (_, graph, top) = scenes()
        .into_iter()
        .find(|s| s.0 == "blend with mask")
        .expect("scene");
    let display = ColorTransform::between(ColorSpace::LINEAR_SRGB, ColorSpace::SRGB);
    let reqs: Vec<_> = [
        region(0, 0, 64, 64),
        region(64, 0, 64, 64),
        region(0, 64, 37, 21),
    ]
    .into_iter()
    .map(|r| RenderRequest::new(top, r).precision(Precision::F32))
    .collect();
    let a = cpu.render_rgba8(&graph, &reqs, &display).expect("cpu");
    let b = gpu
        .render_rgba8(
            &graph,
            &reqs
                .iter()
                .map(|r| r.backend(BackendChoice::Gpu))
                .collect::<Vec<_>>(),
            &display,
        )
        .expect("gpu");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.len(), y.len());
        let worst = x
            .iter()
            .zip(y)
            .map(|(p, q)| p.abs_diff(*q))
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "display tiles differ by {worst}");
    }
}

#[test]
fn icc_sources_enter_the_working_space() {
    let engine = Engine::cpu_only();
    let px = vellum_gpu::SourcePixels::rgba8(
        IRect::of_size(2, 1),
        vellum_gpu::AlphaMode::Straight,
        vec![255, 0, 0, 255, 128, 128, 128, 255],
    );
    let p3 = Arc::new(TiledImage::from_pixels(
        &px,
        SourceColor::Icc(IccProfile::display_p3()),
    ));
    let srgb = Arc::new(TiledImage::from_pixels(
        &px,
        SourceColor::Space(ColorSpace::DISPLAY_P3),
    ));
    let mut g = Graph::new();
    let a = g.add(Node::of(nodes::ImageNode).source(p3));
    let b = g.add(Node::of(nodes::ImageNode).source(srgb));
    let ra = engine
        .render_image(&g, &RenderRequest::new(a, IRect::of_size(2, 1)))
        .expect("icc");
    let rb = engine
        .render_image(&g, &RenderRequest::new(b, IRect::of_size(2, 1)))
        .expect("analytic");
    // p3 red is outside srgb, so linear srgb red goes above one and green below zero
    assert!(ra.get(0, 0)[0] > 1.0 && ra.get(0, 0)[1] < 0.0);
    assert_close("icc vs analytic p3", &ra, &rb, 6e-3);
}

#[test]
fn blend_modes_can_be_named_in_graphs() {
    for mode in BlendMode::ALL {
        let node = Node::of(nodes::BlendNode).param("mode", mode_param(mode));
        let p = vellum_gpu::params::Params {
            specs: node.kind.info().params,
            values: &node.params,
        };
        assert_eq!(p.choice_index("mode"), mode.id(), "{mode:?}");
    }
}

#[test]
fn every_builtin_is_registered_with_sane_parameters() {
    let r = vellum_gpu::NodeRegistry::with_builtins();
    let infos = r.infos();
    assert_eq!(infos.len(), 13);
    for info in infos {
        assert!(info.description.ends_with('.'), "{}", info.id);
        assert!(!info.description.contains('\u{2014}'), "{}", info.id);
        for p in info.params {
            let d = p.default_value();
            assert_eq!(p.sanitize(d.clone()), d, "{}.{}", info.id, p.id);
        }
        let node = r.create(info.id).expect("create");
        assert_eq!(node.inputs.len(), info.inputs.len());
    }
}

#[test]
fn cycles_are_refused() {
    let engine = Engine::cpu_only();
    let mut g = Graph::new();
    let a = g.add(Node::of(nodes::Levels));
    let b = g.add(Node::of(nodes::Levels).input(0, a));
    g.node_mut(a).expect("a").inputs[0] = Some(b);
    let err = engine
        .render_image(&g, &RenderRequest::new(b, region(0, 0, 4, 4)))
        .err();
    assert!(matches!(err, Some(RenderError::Cycle(_))), "{err:?}");
}
