//! layered photoshop import, checked against files from the test writer in
//! `common/psd_writer.rs`. the composites are golden images in `tests/golden/image/psd/`;
//! run with `VELLUM_UPDATE_GOLDEN=1` to write new ones after a deliberate change.

mod common;

use std::path::PathBuf;

use common::psd_writer::{Packing, WDoc, WLayer, WMask, write};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::{BackendChoice, BlendMode, Engine, IRect, Precision};
use vellum_image::compose::{Preview, build};
use vellum_image::document::{Content, ImageDocument, Label, Layer};
use vellum_image::psd;
use vellum_image::raster::Depth;

const W: u32 = 48;
const H: u32 = 40;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/image/psd")
        .join(format!("{name}.png"))
}

/// background, a pass through group holding a masked multiply layer and a layer clipped
/// to it, a hidden layer, and a type layer at half fill with a label and locks
fn layers() -> Vec<WLayer> {
    let bg = WLayer::pixels("Background", (0, 0, W, H), |x, y| {
        [x as f32 / W as f32, y as f32 / H as f32, 0.6, 1.0]
    });
    let mut multiply = WLayer::pixels("Shadow", (8, 6, 24, 20), |x, y| {
        let inside = (x as f32 - 12.0).hypot(y as f32 - 10.0) < 10.0;
        [0.9, 0.4, 0.2, if inside { 1.0 } else { 0.0 }]
    });
    multiply.blend = *b"mul ";
    multiply.opacity = 200;
    multiply.mask = Some(WMask {
        rect: (8, 6, 12, 20),
        values: (0..12 * 20).map(|i| (i % 12) as f32 / 11.0).collect(),
        default: 255,
        disabled: false,
    });
    let mut clipped = WLayer::pixels("Stripes", (0, 0, W, H), |x, _| {
        if (x / 4) % 2 == 0 {
            [0.1, 0.8, 0.3, 1.0]
        } else {
            [0.0; 4]
        }
    });
    clipped.clipped = true;
    clipped.blend = *b"scrn";
    let mut hidden = WLayer::pixels("Hidden", (0, 0, 10, 10), |_, _| [1.0, 0.0, 1.0, 1.0]);
    hidden.hidden = true;
    let mut text = WLayer::pixels("Title", (20, 24, 20, 8), |x, _| {
        [1.0, 1.0, 1.0, if x % 3 == 0 { 1.0 } else { 0.0 }]
    });
    text.text = true;
    text.fill = Some(128);
    text.label = 5;
    text.locks = 4;
    vec![
        bg,
        WLayer::group_end(),
        multiply,
        clipped,
        WLayer::group("Group 1", *b"pass", true),
        hidden,
        text,
    ]
}

fn sample(bits: u16, big: bool, packing: Packing) -> WDoc {
    WDoc {
        width: W,
        height: H,
        bits,
        big,
        gray: false,
        layers: layers(),
        guides: vec![(10 * 32, 0), (20 * 32 + 16, 1)],
        icc: None,
        resolution: Some(300.0),
        packing,
        tagged_layers: bits > 8,
        merged: Vec::new(),
    }
}

fn open(bytes: Vec<u8>) -> ImageDocument {
    psd::read(std::io::Cursor::new(bytes)).expect("the file opens")
}

fn render(doc: &ImageDocument) -> Vec<u16> {
    let (graph, node) = build(doc, &Preview::default());
    let req = RenderRequest::new(node, doc.canvas())
        .working(doc.working())
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    let img = Engine::cpu_only()
        .render_image(&graph, &req)
        .expect("render");
    img.data
        .iter()
        .flat_map(|c| {
            let s = if c[3] > 0.0 {
                [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
            } else {
                [0.0; 4]
            };
            s.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
        })
        .collect()
}

fn check(name: &str, doc: &ImageDocument, tolerance: u16) {
    let stored = render(doc);
    let path = golden(name);
    if std::env::var_os("VELLUM_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("folder")).expect("folder");
        let img: image::ImageBuffer<image::Rgba<u16>, Vec<u16>> =
            image::ImageBuffer::from_raw(doc.width, doc.height, stored).expect("size");
        img.save(&path).expect("write golden");
        return;
    }
    assert!(
        path.exists(),
        "no golden image {}; run with VELLUM_UPDATE_GOLDEN=1",
        path.display()
    );
    let golden = image::open(&path).expect("read").to_rgba16().into_raw();
    let worst = golden
        .iter()
        .zip(&stored)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= tolerance,
        "{name} differs from {} by {worst}/65535",
        path.display()
    );
}

fn find<'a>(doc: &'a ImageDocument, name: &str) -> &'a Layer {
    doc.all_layers()
        .into_iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("no layer called {name}"))
}

#[test]
fn layers_groups_and_settings_survive() {
    let doc = open(write(&sample(8, false, Packing::Rle)));
    assert_eq!((doc.width, doc.height, doc.depth), (W, H, Depth::U8));
    let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Group 1", "Hidden", "Title"]);

    let group = find(&doc, "Group 1");
    match &group.content {
        Content::Group {
            children,
            pass_through,
            expanded,
        } => {
            assert!(*pass_through && *expanded);
            let kids: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(kids, ["Shadow", "Stripes"]);
        }
        Content::Pixels(_) => panic!("Group 1 should be a group"),
    }
    let shadow = find(&doc, "Shadow");
    assert_eq!(shadow.blend, BlendMode::Multiply);
    assert!((shadow.opacity - 200.0 / 255.0).abs() < 1e-6);
    let mask = shadow.mask.as_ref().expect("a mask");
    assert!(mask.enabled);
    assert_eq!(mask.raster.fill(), 1.0);
    assert!(mask.raster.get(8, 10, 0) < 0.01 && mask.raster.get(19, 10, 0) > 0.99);
    assert_eq!(shadow.raster().expect("pixels").content_bounds().x, 11);

    let stripes = find(&doc, "Stripes");
    assert!(stripes.clipped);
    assert_eq!(stripes.blend, BlendMode::Screen);
    assert!(!find(&doc, "Hidden").visible);

    let title = find(&doc, "Title");
    assert_eq!(title.origin.as_deref(), Some("text"));
    assert!((title.fill - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(title.label, Label::Blue);
    assert!(title.locks.position && !title.locks.pixels);

    assert_eq!(doc.guides.len(), 2);
    assert!(doc.guides[0].vertical && doc.guides[0].position == 10.0);
    assert!(!doc.guides[1].vertical && doc.guides[1].position == 20.5);
    assert_eq!(doc.resolution, 300.0);
    assert_eq!(doc.active, Some(title.id));
    check("rgb8_rle", &doc, 2);
}

#[test]
fn every_compression_and_depth_reads_the_same_picture() {
    // the same layers at 8 bits in every compression look exactly alike
    for packing in [Packing::Raw, Packing::Zip, Packing::ZipPredicted] {
        let doc = open(write(&sample(8, false, packing)));
        check("rgb8_rle", &doc, 2);
    }
    // psb is the same file with wider lengths
    let doc = open(write(&sample(8, true, Packing::Rle)));
    check("rgb8_rle", &doc, 2);

    // 16 bit layers live in an Lr16 block; they round to the same picture
    for packing in [Packing::Rle, Packing::ZipPredicted] {
        let d = open(write(&sample(16, false, packing)));
        assert_eq!(d.depth, Depth::U16);
        check("rgb16", &d, 2);
    }
    let d = open(write(&sample(16, true, Packing::Zip)));
    check("rgb16", &d, 2);

    // 32 bit is linear light, so it composites differently and has its own picture
    for (big, packing) in [(false, Packing::ZipPredicted), (true, Packing::Raw)] {
        let d = open(write(&sample(32, big, packing)));
        assert_eq!(d.depth, Depth::F32);
        check("rgb32", &d, 2);
    }
}

#[test]
fn grayscale_profiles_and_flattened_files() {
    // grey layers come in as grey colour
    let mut g = sample(8, false, Packing::Rle);
    g.gray = true;
    g.layers = vec![WLayer::pixels("Background", (0, 0, W, H), |x, _| {
        let v = x as f32 / W as f32;
        [v, v, v, 1.0]
    })];
    let d = open(write(&g));
    let r = find(&d, "Background").raster().expect("pixels");
    let px = r.read(IRect::new(24, 3, 1, 1));
    assert!((px[0] - 0.5).abs() < 0.01 && px[0] == px[1] && px[1] == px[2]);

    // an embedded profile is kept
    let mut p = sample(8, false, Packing::Rle);
    p.icc = Some(
        moxcms::ColorProfile::new_display_p3()
            .encode()
            .expect("profile"),
    );
    let d = open(write(&p));
    assert!(d.color.icc.is_some());
    assert!(d.color.name.is_some());

    // a file saved without layers opens as one layer from the flattened picture
    let mut f = sample(8, false, Packing::Rle);
    f.layers.clear();
    f.merged = (0..W * H)
        .map(|i| [(i % W) as f32 / W as f32, 0.25, 0.75, 1.0])
        .collect();
    let d = open(write(&f));
    assert_eq!(d.layers.len(), 1);
    let px = d.layers[0]
        .raster()
        .expect("pixels")
        .read(IRect::new(12, 0, 1, 1));
    assert!((px[0] - 0.25).abs() < 0.01 && (px[2] - 0.75).abs() < 0.01);
}

#[test]
fn broken_files_say_so_instead_of_crashing() {
    let bytes = write(&sample(8, false, Packing::Rle));
    for cut in [10, 40, bytes.len() / 2, bytes.len() - 7] {
        // a cut file either opens what it can or says it is damaged, and never panics
        let _ = psd::read(std::io::Cursor::new(bytes[..cut].to_vec()));
    }
    let mut cmyk = bytes.clone();
    cmyk[25] = 4;
    let err = psd::read(std::io::Cursor::new(cmyk)).expect_err("cmyk is refused");
    assert!(err.contains("CMYK"));
}
