//! saving photoshop files: every layer kind is written, then read back with the layered
//! reader and checked against the original document, at every depth, as psd and psb.

use vellum_gpu::{BlendMode, IRect};
use vellum_image::adjustments::Adjustment;
use vellum_image::document::{Content, ImageDocument, Label, Layer, LayerMask, Locks};
use vellum_image::flatten;
use vellum_image::psd;
use vellum_image::psd_write;
use vellum_image::raster::{Depth, Raster};
use vellum_image::styles::{LayerStyle, default_shadow};
use vellum_image::vector::{Geometry, PathOp, ShapeItem, ShapeLayer};

const W: u32 = 80;
const H: u32 = 60;

fn layer(doc: &mut ImageDocument, name: &str, rect: IRect, color: [f32; 4]) -> Layer {
    let id = doc.new_id();
    let mut r = Raster::rgba(doc.depth);
    r.update(rect, |x, _, p| {
        let t = x as f32 / W as f32;
        p.copy_from_slice(&[color[0] * (0.5 + t * 0.5), color[1], color[2], color[3]]);
    });
    Layer::pixels(id, name, r)
}

/// a document with a bit of everything
fn sample(depth: Depth) -> ImageDocument {
    let mut doc = ImageDocument::new("export", W, H, depth, Some([0.9, 0.9, 0.85, 1.0]));
    let mut red = layer(
        &mut doc,
        "Red",
        IRect::new(5, 5, 30, 30),
        [0.9, 0.1, 0.1, 1.0],
    );
    red.blend = BlendMode::Multiply;
    red.opacity = 0.8;
    red.fill = 0.6;
    red.label = Label::Green;
    red.locks = Locks {
        position: true,
        ..Default::default()
    };
    let mut mask = Raster::gray(depth, 1.0);
    mask.update(IRect::new(5, 5, 10, 30), |_, _, v| v[0] = 0.0);
    red.mask = Some(LayerMask {
        raster: mask,
        enabled: true,
        linked: true,
    });
    let mut clipped = layer(
        &mut doc,
        "Clipped",
        IRect::new(0, 15, 80, 10),
        [0.1, 0.2, 0.9, 1.0],
    );
    clipped.clipped = true;
    let mut inner = layer(
        &mut doc,
        "Inner é",
        IRect::new(40, 30, 30, 25),
        [0.1, 0.8, 0.2, 0.7],
    );
    inner.visible = true;
    let gid = doc.new_id();
    let mut group = Layer::group(gid, "Group", vec![inner]);
    group.opacity = 0.9;
    let sid = doc.new_id();
    let shape = Layer {
        content: Content::Shape {
            shape: Box::new(ShapeLayer {
                items: vec![ShapeItem {
                    geometry: Geometry::Ellipse {
                        x: 45.0,
                        y: 5.0,
                        w: 25.0,
                        h: 20.0,
                    },
                    op: PathOp::Combine,
                }],
                fill: Some([1.0, 0.8, 0.1, 1.0]),
                stroke: None,
            }),
            pixels: Raster::rgba(depth),
        },
        style: Some(LayerStyle {
            drop_shadow: Some(default_shadow()),
            ..Default::default()
        }),
        ..Layer::pixels(sid, "Sun", Raster::rgba(depth))
    };
    let lid = doc.new_id();
    let mut levels = Layer {
        content: Content::Adjustment(Adjustment::new("effect.levels").with("gamma", 1.4f32)),
        ..Layer::pixels(lid, "Levels", Raster::rgba(depth))
    };
    levels.opacity = 0.7;
    let cid = doc.new_id();
    let curves = Layer {
        content: Content::Adjustment(Adjustment::new("effect.vibrance").with("vibrance", 60.0f32)),
        ..Layer::pixels(cid, "Vibrance", Raster::rgba(depth))
    };
    doc.layers
        .extend([red, clipped, group, shape, levels, curves]);
    // draw the shape's pixels the way the editor would
    let f = vellum_image::vector_ops::to_doc(&doc);
    let canvas = doc.canvas();
    if let Some(l) = doc.layer_mut(sid) {
        vellum_image::vector_ops::redraw(l, depth, canvas, &f);
    }
    doc
}

fn names(list: &[Layer], out: &mut Vec<String>) {
    for l in list {
        out.push(l.name.clone());
        names(l.children(), out);
    }
}

fn worst(a: &Raster, b: &Raster) -> f32 {
    let r = IRect::of_size(W, H);
    let (pa, pb) = (a.read(r), b.read(r));
    pa.iter()
        .zip(&pb)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn every_depth_round_trips_through_psd_and_psb() {
    for depth in [Depth::U8, Depth::U16, Depth::F32] {
        for big in [false, true] {
            let doc = sample(depth);
            let bytes = psd_write::encode(&doc, big).expect("encode");
            assert_eq!(&bytes[..4], b"8BPS");
            let back = psd::read(std::io::Cursor::new(bytes)).expect("read back");
            let label = format!("{depth:?} big={big}");
            assert_eq!(
                (back.width, back.height, back.depth),
                (W, H, depth),
                "{label}"
            );

            let mut a = Vec::new();
            names(&doc.layers, &mut a);
            let mut b = Vec::new();
            names(&back.layers, &mut b);
            assert_eq!(
                b,
                vec![
                    "Background",
                    "Red",
                    "Clipped",
                    "Group",
                    "Inner é",
                    "Sun",
                    "Levels",
                    "Vibrance (applied)"
                ],
                "{label}"
            );
            let red = &back.layers[1];
            assert_eq!(red.blend, BlendMode::Multiply, "{label}");
            assert!((red.opacity - 0.8).abs() < 0.01 && (red.fill - 0.6).abs() < 0.01);
            assert_eq!(red.label, Label::Green);
            assert!(red.locks.position);
            assert!(red.mask.is_some(), "{label}");
            assert!(back.layers[2].clipped);
            assert!(matches!(
                back.layers[3].content,
                Content::Group {
                    pass_through: true,
                    ..
                }
            ));

            // the picture the layers make is the same, adjustment and style included
            let before = flatten::merged(&doc, doc.canvas(), Depth::F32).expect("before");
            let after = flatten::merged(&back, back.canvas(), Depth::F32).expect("after");
            let d = worst(&before, &after);
            assert!(d < 0.03, "{label}: the picture changed by {d}");
        }
    }
}

#[test]
fn native_adjustments_come_back_as_adjustments() {
    // without the baked layer on top, so the levels layer's effect shows in the picture
    let mut doc = sample(Depth::U8);
    doc.layers.pop();
    for (kind, key, value) in [
        ("effect.invert", None, None),
        (
            "effect.posterize",
            Some("levels"),
            Some(vellum_gpu::ParamValue::Int(5)),
        ),
        (
            "effect.threshold",
            Some("level"),
            Some(vellum_gpu::ParamValue::Float(100.0)),
        ),
    ] {
        let id = doc.new_id();
        let mut a = Adjustment::new(kind);
        if let (Some(k), Some(v)) = (key, value) {
            a.params.insert(k.into(), v);
        }
        let mut l = Layer {
            content: Content::Adjustment(a),
            ..Layer::pixels(id, kind, Raster::rgba(Depth::U8))
        };
        l.visible = false;
        doc.layers.push(l);
    }
    let bytes = psd_write::encode(&doc, false).expect("encode");
    assert!(bytes.windows(8).any(|w| w == b"8BIMlevl"));
    let back = psd::read(std::io::Cursor::new(bytes)).expect("read");
    let kinds: Vec<&str> = back
        .layers
        .iter()
        .filter_map(|l| match &l.content {
            Content::Adjustment(a) => Some(a.kind.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "effect.levels",
            "effect.invert",
            "effect.posterize",
            "effect.threshold"
        ]
    );
    let before = flatten::merged(&doc, doc.canvas(), Depth::F32).expect("before");
    let after = flatten::merged(&back, back.canvas(), Depth::F32).expect("after");
    let d = worst(&before, &after);
    assert!(
        d < 0.02,
        "the levels layer changes the picture the same way: {d}"
    );
}

#[test]
fn saving_writes_a_file_through_a_temporary_name() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("out.psd");
    psd_write::save(&sample(Depth::U8), &path).expect("save");
    let back = psd::open(&path).expect("open");
    assert_eq!(back.layers.len(), 7);
    // VELLUM_PSD_DUMP names a folder to leave sample files in, for checking them in
    // other apps (photoshop, photopea, psd-tools, imagemagick)
    if let Some(dir) = std::env::var_os("VELLUM_PSD_DUMP") {
        let dir = std::path::PathBuf::from(dir);
        for (depth, name) in [(Depth::U8, "8"), (Depth::U16, "16"), (Depth::F32, "32")] {
            psd_write::save(&sample(depth), &dir.join(format!("sample-{name}.psd"))).expect("dump");
        }
        psd_write::save(&sample(Depth::U8), &dir.join("sample-8.psb")).expect("dump");
    }
    let mut huge = ImageDocument::new("huge", 30_001, 2, Depth::U8, None);
    huge.layers.clear();
    assert!(psd_write::encode(&huge, false).is_err());
}
