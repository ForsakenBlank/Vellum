//! exports beyond the whole picture: each layer to its own file, each artboard to its own
//! file, and save for web, which encodes the picture and decodes it again so the window
//! can show the result next to the original with its real file size.
//!
//! all of them reuse the normal export (`export.rs`) on a copy of the document cut down to
//! what is wanted, so formats, profiles and export to a size behave the same everywhere.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_export::Env;
use vellum_gpu::{BlendMode, IRect};
use vellum_media::Control;

use crate::canvas::Crop;
use crate::commands::Edit;
use crate::document::{Artboard, Content, ImageDocument, Layer, LayerId};
use crate::export::{ExportFormat, ExportOptions, export};

/// a file name from a layer name: letters, digits, spaces, dashes and underscores
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('_');
    if trimmed.is_empty() {
        "layer".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} {n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// the document cut down to a box
fn cropped(doc: &ImageDocument, rect: IRect) -> Result<ImageDocument, String> {
    let mut d = doc.clone();
    d.selection = None;
    d.quick_mask = false;
    if rect != d.canvas() {
        Crop {
            rect: [rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32],
            angle: 0.0,
            perspective: None,
            delete_outside: true,
            interpolation: Default::default(),
        }
        .run(&mut d)?;
    }
    Ok(d)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LayerExport {
    pub options: ExportOptions,
    /// cut each file to its layer's pixels instead of the whole canvas
    pub trim: bool,
    /// include hidden layers
    pub hidden: bool,
    /// each layer of a group on its own, instead of the group as one picture
    pub inside_groups: bool,
    /// put the layer's number before its name, so files sort in layer order
    pub numbered: bool,
}

impl Default for LayerExport {
    fn default() -> Self {
        Self {
            options: ExportOptions::default(),
            trim: true,
            hidden: false,
            inside_groups: false,
            numbered: true,
        }
    }
}

/// the layers export to files would write, top to bottom like the layers panel
pub fn layers_to_export(doc: &ImageDocument, how: &LayerExport) -> Vec<LayerId> {
    fn walk(list: &[Layer], how: &LayerExport, out: &mut Vec<LayerId>) {
        for l in list.iter().rev() {
            if !l.visible && !how.hidden {
                continue;
            }
            if matches!(l.content, Content::Adjustment(_)) {
                continue;
            }
            if l.is_group() && how.inside_groups {
                walk(l.children(), how, out);
            } else {
                out.push(l.id);
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, how, &mut out);
    out
}

/// the document showing only one layer, as it looks on its own
fn alone(doc: &ImageDocument, id: LayerId) -> Result<ImageDocument, String> {
    let layer = doc.layer(id).ok_or("That layer is gone.")?.clone();
    let mut d = doc.clone();
    d.selection = None;
    d.quick_mask = false;
    d.layers = vec![Layer {
        visible: true,
        clipped: false,
        blend: BlendMode::Normal,
        ..layer
    }];
    Ok(d)
}

/// writes each layer to its own file in `dir`. `progress` gets the share done.
pub fn export_layers(
    doc: &ImageDocument,
    dir: &Path,
    how: &LayerExport,
    env: &Env,
    margin: f64,
    control: &dyn Control,
    mut progress: impl FnMut(f32),
) -> Result<Vec<PathBuf>, String> {
    let ids = layers_to_export(doc, how);
    if ids.is_empty() {
        return Err("There are no layers to export. Show them, or include hidden layers.".into());
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("The folder could not be made: {err}"))?;
    let ext = how.options.format.extension();
    let mut out = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        control.checkpoint().map_err(|e| e.to_string())?;
        progress(i as f32 / ids.len() as f32);
        let solo = alone(doc, *id)?;
        let layer = &solo.layers[0];
        let rect = if how.trim {
            let b = crate::flatten::merged(&solo, solo.canvas().expand(512), solo.depth)?
                .content_bounds();
            if b.is_empty() {
                continue;
            }
            b
        } else {
            solo.canvas()
        };
        let piece = cropped(&solo, rect)?;
        let stem = if how.numbered {
            format!("{:02} {}", i + 1, safe_name(&layer.name))
        } else {
            safe_name(&layer.name)
        };
        let path = unique(dir, &stem, ext);
        export(&piece, &path, &how.options, env, margin, control)?;
        out.push(path);
    }
    progress(1.0);
    Ok(out)
}

/// every artboard in the document, top to bottom
pub fn artboards(doc: &ImageDocument) -> Vec<(LayerId, String, Artboard)> {
    doc.all_layers()
        .into_iter()
        .rev()
        .filter_map(|l| l.artboard.map(|a| (l.id, l.name.clone(), a)))
        .collect()
}

/// writes each artboard to its own file in `dir`
pub fn export_artboards(
    doc: &ImageDocument,
    dir: &Path,
    options: &ExportOptions,
    env: &Env,
    margin: f64,
    control: &dyn Control,
    mut progress: impl FnMut(f32),
) -> Result<Vec<PathBuf>, String> {
    let boards = artboards(doc);
    if boards.is_empty() {
        return Err("This document has no artboards. Make one from the Layer menu first.".into());
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("The folder could not be made: {err}"))?;
    let mut out = Vec::new();
    for (i, (id, name, board)) in boards.iter().enumerate() {
        control.checkpoint().map_err(|e| e.to_string())?;
        progress(i as f32 / boards.len() as f32);
        let mut solo = doc.clone();
        // only this artboard: other artboards hide, loose layers stay as they are
        fn hide_others(list: &mut [Layer], keep: LayerId) {
            for l in list {
                if l.artboard.is_some() && l.id != keep {
                    l.visible = false;
                }
                if let Some(c) = l.children_mut() {
                    hide_others(c, keep);
                }
            }
        }
        hide_others(&mut solo.layers, *id);
        if let Some(l) = solo.layer_mut(*id) {
            l.visible = true;
        }
        let piece = cropped(&solo, board.rect)?;
        let path = unique(dir, &safe_name(name), options.format.extension());
        export(&piece, &path, options, env, margin, control)?;
        out.push(path);
    }
    progress(1.0);
    Ok(out)
}

/// makes a group into an artboard, or a new empty one
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetArtboard {
    /// the group; none makes a new artboard group
    pub id: Option<LayerId>,
    /// none turns the artboard back into a plain group
    pub artboard: Option<Artboard>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Edit for SetArtboard {
    const KIND: &'static str = "image.artboard.set";
    const TITLE: &'static str = "Artboard";
    const DESCRIPTION: &'static str = "Make, resize or remove an artboard.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if let Some(a) = &self.artboard
            && a.rect.is_empty()
        {
            return Err("An artboard needs a size.".into());
        }
        match self.id {
            Some(id) => {
                let layer = doc.layer_mut(id).ok_or("That layer is gone.")?;
                if !layer.is_group() {
                    return Err("Only groups can become artboards. Group the layers first.".into());
                }
                layer.artboard = self.artboard;
                if let Some(n) = &self.name {
                    layer.name = n.clone();
                }
            }
            None => {
                let artboard = self.artboard.ok_or("Draw the artboard first.")?;
                let id = doc.new_id();
                let name = self
                    .name
                    .clone()
                    .unwrap_or_else(|| doc.fresh_name("Artboard"));
                let mut group = Layer::group(id, name, Vec::new());
                if let crate::document::Content::Group { pass_through, .. } = &mut group.content {
                    *pass_through = false;
                }
                group.artboard = Some(artboard);
                doc.layers.push(group);
                doc.active = Some(id);
                self.id = Some(id);
            }
        }
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id.is_some()
            && self.id == next.id
            && self.artboard.is_some()
            && next.artboard.is_some()
    }
}

// ---- save for web ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct WebOptions {
    pub format: ExportFormat,
    /// 1 to 100
    pub quality: u8,
    /// percent of the full size
    pub scale: f32,
    /// straight srgb behind see through pixels for jpeg
    pub background: [f32; 3],
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jpeg,
            quality: 70,
            scale: 100.0,
            background: [1.0, 1.0, 1.0],
        }
    }
}

/// one side of the comparison: straight srgb rgba8 of the part on view
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebPreview {
    /// the encoded file's size
    #[ts(type = "number")]
    pub bytes: u64,
    /// the size of the picture after scaling
    pub width: u32,
    pub height: u32,
    /// the part shown, in scaled pixels
    pub view: IRect,
    /// the encoded file decoded again, over `view`
    pub optimized: Vec<u8>,
    /// the picture before encoding, over `view`
    pub original: Vec<u8>,
}

fn rgba8_of(img: &image::RgbaImage, view: IRect) -> Vec<u8> {
    let mut out = vec![0u8; view.area() as usize * 4];
    for y in 0..view.h {
        for x in 0..view.w {
            let (sx, sy) = (view.x as u32 + x, view.y as u32 + y);
            if sx < img.width() && sy < img.height() {
                let p = img.get_pixel(sx, sy).0;
                let i = ((y * view.w + x) * 4) as usize;
                out[i..i + 4].copy_from_slice(&p);
            }
        }
    }
    out
}

/// encodes the picture with the web settings and decodes it again, for the side by side
/// comparison. `view` is the part to send back, in scaled pixels.
pub fn web_preview(
    doc: &ImageDocument,
    opts: &WebOptions,
    view: IRect,
    env: &Env,
    control: &dyn Control,
) -> Result<WebPreview, String> {
    let mut d = doc.clone();
    d.selection = None;
    d.quick_mask = false;
    let k = (opts.scale / 100.0).clamp(0.01, 1.0);
    if (k - 1.0).abs() > 1e-4 {
        let w = ((doc.width as f32 * k).round() as u32).max(1);
        let h = ((doc.height as f32 * k).round() as u32).max(1);
        crate::canvas::ImageSize {
            width: w,
            height: h,
            resolution: None,
            interpolation: Default::default(),
        }
        .run(&mut d)?;
    }
    let temp = tempfile::tempdir().map_err(|err| format!("No temporary folder: {err}"))?;
    let path = temp.path().join(format!("web.{}", opts.format.extension()));
    let options = ExportOptions {
        format: opts.format,
        quality: opts.quality.clamp(1, 100),
        target: None,
        background: opts.background,
        embed_profile: false,
    };
    export(&d, &path, &options, env, 0.0, control)?;
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let decoded = vellum_media::picture::decode(&path, Default::default(), env.ffmpeg)
        .map_err(|err| format!("The result could not be read back: {err}"))?;
    let optimized = decoded.first().clone();
    let original_path = temp.path().join("original.png");
    export(
        &d,
        &original_path,
        &ExportOptions {
            format: ExportFormat::Png,
            embed_profile: false,
            ..Default::default()
        },
        env,
        0.0,
        control,
    )?;
    let original = image::open(&original_path)
        .map_err(|err| format!("The original could not be read back: {err}"))?
        .to_rgba8();
    let full = IRect::of_size(d.width, d.height);
    let view = if view.is_empty() {
        full
    } else {
        view.intersect(&full)
    };
    Ok(WebPreview {
        bytes,
        width: d.width,
        height: d.height,
        view,
        optimized: rgba8_of(&optimized, view),
        original: rgba8_of(&original, view),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Depth, Raster};

    struct Go;
    impl Control for Go {
        fn checkpoint(&self) -> Result<(), vellum_media::MediaError> {
            Ok(())
        }
        fn progress(&self, _done: f32) {}
    }

    fn doc() -> ImageDocument {
        let mut doc = ImageDocument::new("t", 60, 40, Depth::U8, Some([1.0; 4]));
        for (i, x) in [5, 30].iter().enumerate() {
            let id = doc.new_id();
            let mut r = Raster::rgba(Depth::U8);
            r.update(IRect::new(*x, 5, 20, 20), |_, _, p| {
                p.copy_from_slice(&[1.0, 0.2, 0.1, 1.0])
            });
            doc.layers
                .push(Layer::pixels(id, format!("Shape {}/{i}", i + 1), r));
        }
        doc
    }

    fn env_parts() -> vellum_media::ffmpeg::hw::HardwareSupport {
        Default::default()
    }

    #[test]
    fn names_are_safe_and_unique() {
        assert_eq!(safe_name("Logo / dark?"), "Logo _ dark");
        assert_eq!(safe_name("///"), "layer");
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("a.png"), b"x").expect("write");
        assert_eq!(
            unique(dir.path(), "a", "png").file_name().unwrap(),
            "a 2.png"
        );
    }

    #[test]
    fn layers_export_trimmed_in_panel_order() {
        let d = doc();
        let dir = tempfile::tempdir().expect("dir");
        let hw = env_parts();
        let env = Env {
            ffmpeg: None,
            hardware: &hw,
            allow_hardware: false,
            output_folder: None,
        };
        let files = export_layers(
            &d,
            dir.path(),
            &LayerExport::default(),
            &env,
            0.0,
            &Go,
            |_| {},
        )
        .expect("export");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["01 Shape 2_1.png", "02 Shape 1_0.png", "03 Background.png"]
        );
        let img = image::open(&files[0]).expect("open").to_rgba8();
        assert_eq!(img.dimensions(), (20, 20), "trimmed to the layer");
    }

    #[test]
    fn artboards_clip_and_export_on_their_own() {
        let mut d = doc();
        let mut cmd = SetArtboard {
            id: None,
            artboard: Some(Artboard {
                rect: IRect::new(0, 0, 30, 40),
                background: Some([0.0, 0.0, 1.0, 1.0]),
            }),
            name: Some("Phone".into()),
        };
        cmd.run(&mut d).expect("artboard");
        let px = crate::flatten::colors(&d, IRect::new(2, 30, 1, 1))
            .expect("c")
            .get(2, 30);
        assert!(
            px[2] > 0.9 && px[0] < 0.1,
            "the artboard's blue background: {px:?}"
        );
        let outside = crate::flatten::colors(&d, IRect::new(45, 30, 1, 1))
            .expect("c")
            .get(45, 30);
        assert!(outside[0] > 0.9, "nothing drawn past the board");
        let dir = tempfile::tempdir().expect("dir");
        let hw = env_parts();
        let env = Env {
            ffmpeg: None,
            hardware: &hw,
            allow_hardware: false,
            output_folder: None,
        };
        let files = export_artboards(
            &d,
            dir.path(),
            &ExportOptions::default(),
            &env,
            0.0,
            &Go,
            |_| {},
        )
        .expect("export");
        assert_eq!(files.len(), 1);
        assert_eq!(
            image::open(&files[0])
                .expect("open")
                .to_rgba8()
                .dimensions(),
            (30, 40)
        );
    }

    #[test]
    fn save_for_web_shows_both_sides_and_the_size() {
        let d = doc();
        let hw = env_parts();
        let env = Env {
            ffmpeg: None,
            hardware: &hw,
            allow_hardware: false,
            output_folder: None,
        };
        let low = WebOptions {
            quality: 10,
            ..Default::default()
        };
        let high = WebOptions {
            quality: 95,
            ..Default::default()
        };
        let a = web_preview(&d, &low, IRect::EMPTY, &env, &Go).expect("low");
        let b = web_preview(&d, &high, IRect::new(0, 0, 30, 20), &env, &Go).expect("high");
        assert!(a.bytes < b.bytes, "{} vs {}", a.bytes, b.bytes);
        assert_eq!(a.optimized.len(), 60 * 40 * 4);
        assert_eq!(b.view, IRect::new(0, 0, 30, 20));
        assert_eq!(b.original.len(), 30 * 20 * 4);
        let half = web_preview(
            &d,
            &WebOptions {
                scale: 50.0,
                ..Default::default()
            },
            IRect::EMPTY,
            &env,
            &Go,
        )
        .expect("half");
        assert_eq!((half.width, half.height), (30, 20));
    }
}
