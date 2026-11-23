//! the viewport sends each tile once and again only when its content changes

mod common;

use std::sync::Arc;
use std::time::Duration;

use vellum_gpu::nodes;
use vellum_gpu::view::{
    FrameUpdate, Picture, ViewSettings, ViewState, Viewport, level_for_zoom, pack,
};
use vellum_gpu::{Engine, Graph, IRect, Node, NodeId, ProceduralSource};

const DOC: IRect = IRect::new(0, 0, 3000, 2000);
const FOREVER: Duration = Duration::from_secs(60);

fn graph(gamma: f32) -> (Graph, NodeId) {
    let mut g = Graph::new();
    let src = Arc::new(ProceduralSource::new(DOC.w, DOC.h, |x, y| {
        [
            (x * 0.01).sin() * 0.5 + 0.5,
            (y * 0.01).cos() * 0.5 + 0.5,
            0.5,
            1.0,
        ]
    }));
    let pic = g.add(Node::of(nodes::ImageNode).source(src));
    let top = g.add(Node::of(nodes::Levels).param("gamma", gamma).input(0, pic));
    (g, top)
}

fn view(cx: f64, zoom: f64) -> ViewState {
    ViewState {
        center_x: cx,
        center_y: 1000.0,
        zoom,
        rotation: 0.0,
        flip_x: false,
        width: 1280,
        height: 720,
    }
}

fn frame(
    vp: &mut Viewport,
    engine: &Engine,
    g: &Graph,
    top: NodeId,
    rev: u64,
    v: &ViewState,
) -> FrameUpdate {
    let picture = Picture {
        graph: g,
        node: top,
        revision: rev,
    };
    vp.frame(engine, &picture, v, &ViewSettings::default(), FOREVER)
        .expect("frame")
}

#[test]
fn tiles_are_sent_once_and_again_only_when_they_change() {
    let engine = common::gpu_engine().unwrap_or_else(Engine::cpu_only);
    let (g, top) = graph(1.0);
    let mut vp = Viewport::new(DOC);
    let v = view(1500.0, 1.0);

    let first = frame(&mut vp, &engine, &g, top, 1, &v);
    assert_eq!(first.pending, 0);
    assert_eq!(first.tiles.len(), vp.wanted(&v).len());
    for t in &first.tiles {
        assert_eq!(t.pixels.len(), t.rect.area() as usize * 4);
        assert!(DOC.at_level(t.id.level).contains_rect(&t.rect));
    }

    // nothing changed, nothing sent
    let again = frame(&mut vp, &engine, &g, top, 1, &v);
    assert!(again.tiles.is_empty() && again.dropped.is_empty());

    // panning right only sends the newly exposed column of tiles
    let panned = frame(&mut vp, &engine, &g, top, 1, &view(1800.0, 1.0));
    assert!(!panned.tiles.is_empty());
    assert!(
        panned.tiles.len() < first.tiles.len() / 2,
        "{} tiles",
        panned.tiles.len()
    );

    // zoom out: the window keeps its level 0 tiles for later
    let far = view(1500.0, 0.2);
    let zoomed = frame(&mut vp, &engine, &g, top, 1, &far);
    assert!(
        zoomed
            .tiles
            .iter()
            .all(|t| t.id.level == level_for_zoom(0.2))
    );
    let held = vp.held();

    // an edit resends what is visible and drops level 0 tiles that went stale
    let (g2, top2) = graph(1.6);
    let edited = frame(&mut vp, &engine, &g2, top2, 2, &far);
    assert!(!edited.tiles.is_empty());
    assert!(!edited.dropped.is_empty());
    assert!(edited.dropped.iter().all(|t| t.level == 0));
    assert_eq!(vp.held(), held - edited.dropped.len());
}

#[test]
fn a_tight_budget_spreads_work_over_frames() {
    let engine = Engine::cpu_only();
    let (g, top) = graph(1.0);
    let mut vp = Viewport::new(DOC);
    let v = view(1500.0, 1.0);
    let settings = ViewSettings::default();
    let mut frames = 0;
    let mut sent = 0;
    loop {
        let f = vp
            .frame(
                &engine,
                &Picture {
                    graph: &g,
                    node: top,
                    revision: 1,
                },
                &v,
                &settings,
                Duration::ZERO,
            )
            .expect("frame");
        sent += f.tiles.len();
        frames += 1;
        if f.pending == 0 {
            break;
        }
        assert!(!f.tiles.is_empty(), "every frame makes progress");
    }
    assert!(frames > 1);
    assert_eq!(sent, vp.wanted(&v).len());
}

#[test]
fn forgotten_tiles_are_sent_again() {
    let engine = Engine::cpu_only();
    let (g, top) = graph(1.0);
    let mut vp = Viewport::new(DOC);
    let v = view(1500.0, 1.0);
    let first = frame(&mut vp, &engine, &g, top, 1, &v);
    let gone: Vec<_> = first.tiles.iter().take(3).map(|t| t.id).collect();
    vp.forget(&gone);
    let again = frame(&mut vp, &engine, &g, top, 1, &v);
    let ids: Vec<_> = again.tiles.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 3);
    assert!(gone.iter().all(|id| ids.contains(id)));
    assert!(pack(&again).len() > 3 * 256 * 256 * 4 / 2);
}
