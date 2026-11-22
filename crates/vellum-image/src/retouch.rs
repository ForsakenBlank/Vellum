//! retouching that works on the selection rather than with a brush: the patch tool,
//! content aware move, content aware fill and red eye removal.

use serde::{Deserialize, Serialize};
use vellum_gpu::IRect;

use crate::commands::{Edit, shifted};
use crate::document::ImageDocument;
use crate::heal;
use crate::paint::{active_part, raster_for, set_part};
use crate::raster::Raster;

fn selection(doc: &ImageDocument) -> Result<Raster, String> {
    doc.selection
        .clone()
        .filter(|s| !s.content_bounds().is_empty())
        .ok_or_else(|| "Select the area first.".to_owned())
}

/// patch: heal the selection with texture from somewhere else (source), or copy the
/// selection somewhere else and heal it in there (destination)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Patch {
    /// how far the selection was dragged
    pub dx: i32,
    pub dy: i32,
    /// copy the selection to where it was dragged, instead of patching it from there
    pub destination: bool,
}

impl Edit for Patch {
    const KIND: &'static str = "image.retouch.patch";
    const TITLE: &'static str = "Patch";
    const DESCRIPTION: &'static str = "Repair the selection with texture from another area.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let sel = selection(doc)?;
        let part = active_part(doc)?;
        let base = raster_for(doc, &part)?;
        let area = sel.content_bounds();
        let out = if self.destination {
            let moved = shifted(&sel, self.dx, self.dy);
            heal::heal(
                &base,
                &base,
                -self.dx,
                -self.dy,
                &moved,
                area.translate(self.dx, self.dy),
            )?
        } else {
            heal::heal(&base, &base, self.dx, self.dy, &sel, area)?
        };
        set_part(doc, &part, out);
        Ok(())
    }
}

/// fills the selection with texture from around it
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentAwareFill {}

impl Edit for ContentAwareFill {
    const KIND: &'static str = "image.retouch.fill";
    const TITLE: &'static str = "Content-aware fill";
    const DESCRIPTION: &'static str = "Fill the selection with what is around it.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let sel = selection(doc)?;
        let part = active_part(doc)?;
        let base = raster_for(doc, &part)?;
        let out = heal::content_fill(&base, &sel, doc.canvas())?;
        set_part(doc, &part, out);
        Ok(())
    }
}

/// moves the selected pixels and fills the hole they leave
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentAwareMove {
    pub dx: i32,
    pub dy: i32,
    /// copy instead of move, leaving the original where it was
    pub extend: bool,
}

impl Edit for ContentAwareMove {
    const KIND: &'static str = "image.retouch.move";
    const TITLE: &'static str = "Content-aware move";
    const DESCRIPTION: &'static str = "Move the selection and fill the hole it leaves.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let sel = selection(doc)?;
        let part = active_part(doc)?;
        let base = raster_for(doc, &part)?;
        let filled = if self.extend {
            base.clone()
        } else {
            heal::content_fill(&base, &sel, doc.canvas())?
        };
        // the moved pixels land through their selection and are healed into place a
        // little at the edges, so they take on the new surroundings
        let moved_sel = shifted(&sel, self.dx, self.dy);
        let area = moved_sel.content_bounds();
        let mut out = filled.clone();
        let n = base.channels().n();
        let (dx, dy) = (self.dx, self.dy);
        out.update(area, |x, y, px| {
            let a = moved_sel.get(x, y, 0);
            if a <= 0.0 {
                return;
            }
            for (k, v) in px.iter_mut().enumerate().take(n) {
                *v += (base.get(x - dx, y - dy, k) - *v) * a;
            }
        });
        let mut edge = Raster::gray(crate::raster::Depth::F32, 0.0);
        edge.update(area.expand(2), |x, y, v| {
            let a = moved_sel.get(x, y, 0);
            v[0] = if a > 0.0 && a < 1.0 { 1.0 } else { 0.0 };
        });
        let out = heal::heal(&out, &out, 0, 0, &edge, area.expand(2)).unwrap_or(out);
        set_part(doc, &part, out);
        doc.selection = Some(moved_sel);
        Ok(())
    }
}

/// takes the red out of eyes inside a box
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedEye {
    pub rect: IRect,
    /// 0 to 1, how much of the red area counts as pupil
    pub pupil: f32,
    /// 0 to 1, how dark the pupil becomes
    pub darken: f32,
}

impl Default for RedEye {
    fn default() -> Self {
        Self {
            rect: IRect::EMPTY,
            pupil: 0.5,
            darken: 0.5,
        }
    }
}

/// how red a straight colour is, 0 to 1
pub fn redness(c: [f32; 3]) -> f32 {
    let other = c[1].max(c[2]);
    if c[0] <= 0.05 {
        return 0.0;
    }
    ((c[0] - other) / c[0] - 0.25).clamp(0.0, 0.5) * 2.0
}

impl Edit for RedEye {
    const KIND: &'static str = "image.retouch.red_eye";
    const TITLE: &'static str = "Red eye";
    const DESCRIPTION: &'static str = "Take the red out of eyes lit by a flash.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let rect = self.rect.intersect(&doc.canvas());
        if rect.is_empty() {
            return Err("Drag a box around the eye.".into());
        }
        let part = active_part(doc)?;
        let base = raster_for(doc, &part)?;
        if base.channels() != crate::raster::Channels::Rgba {
            return Err("Red eye works on a layer's colours, not a mask.".into());
        }
        // the pupil is a disc in the middle of the box; red outside it is left alone
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let radius = rect.w.min(rect.h) as f32 / 2.0 * (0.5 + self.pupil.clamp(0.0, 1.0) * 0.5);
        let darken = self.darken.clamp(0.0, 1.0);
        let mut out = base.clone();
        out.update(rect, |x, y, px| {
            let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let inside = (radius - d + 1.0).clamp(0.0, 1.0);
            let red = redness([px[0], px[1], px[2]]) * inside;
            if red <= 0.0 {
                return;
            }
            let g = (px[1] + px[2]) / 2.0 * (1.0 - darken * 0.8);
            for v in px.iter_mut().take(3) {
                *v += (g - *v) * red;
            }
        });
        set_part(doc, &part, out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry;
    use crate::document::Layer;
    use crate::paint::{BrushSettings, BrushTool, Stroke, StrokePoint};
    use crate::raster::Depth;
    use vellum_core::command::CommandRecord;
    use vellum_core::history::{History, HistoryLimits};

    fn doc() -> ImageDocument {
        let mut doc = ImageDocument::new("t", 96, 64, Depth::U8, Some([0.5, 0.5, 0.5, 1.0]));
        let id = doc.layers[0].id;
        doc.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::of_size(96, 64), |x, y, px| {
                let v = if (x / 4 + y / 4) % 2 == 0 { 0.35 } else { 0.55 };
                px.copy_from_slice(&[v, v * 0.95, v * 0.9, 1.0]);
            });
        doc.active = Some(id);
        doc
    }

    fn px(doc: &ImageDocument, x: i32, y: i32) -> [f32; 4] {
        let r = doc.layers[0].raster().expect("raster");
        [0, 1, 2, 3].map(|k| r.get(x, y, k))
    }

    fn stroke(doc: &ImageDocument, settings: BrushSettings, pts: &[[f32; 2]]) -> Raster {
        let mut s = Stroke::new(doc, settings).expect("stroke");
        let pts: Vec<StrokePoint> = pts
            .iter()
            .map(|p| StrokePoint {
                x: p[0],
                y: p[1],
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
            })
            .collect();
        s.add(&pts);
        s.finish();
        s.result().clone()
    }

    fn brush(tool: BrushTool) -> BrushSettings {
        BrushSettings {
            tool,
            size: 12.0,
            hardness: 1.0,
            smoothing: 0.0,
            pressure_size: false,
            ..Default::default()
        }
    }

    #[test]
    fn dodge_burn_and_sponge_move_tones_the_right_way() {
        let d = doc();
        let before = px(&d, 20, 20);
        let lighter = stroke(&d, brush(BrushTool::Dodge), &[[10.0, 20.0], [30.0, 20.0]]);
        let darker = stroke(&d, brush(BrushTool::Burn), &[[10.0, 20.0], [30.0, 20.0]]);
        assert!(lighter.get(20, 20, 0) > before[0]);
        assert!(darker.get(20, 20, 0) < before[0]);
        let mut grey = brush(BrushTool::Sponge);
        grey.retouch.desaturate = true;
        let dull = stroke(&d, grey, &[[10.0, 20.0], [30.0, 20.0]]);
        let spread = |r: &Raster| r.get(20, 20, 0) - r.get(20, 20, 2);
        assert!(spread(&dull) < before[0] - before[2]);
    }

    #[test]
    fn blur_softens_and_sharpen_crisps_an_edge() {
        let d = doc();
        let contrast = |r: &Raster| (r.get(19, 20, 0) - r.get(20, 20, 0)).abs();
        let base = d.layers[0].raster().expect("raster").clone();
        let blurred = stroke(&d, brush(BrushTool::Blur), &[[10.0, 20.0], [30.0, 20.0]]);
        let sharp = stroke(&d, brush(BrushTool::Sharpen), &[[10.0, 20.0], [30.0, 20.0]]);
        assert!(contrast(&blurred) < contrast(&base));
        assert!(contrast(&sharp) > contrast(&base));
    }

    #[test]
    fn clone_copies_from_the_offset_and_smudge_drags_colour() {
        let mut d = doc();
        let id = d.layers[0].id;
        d.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::new(60, 10, 10, 10), |_, _, p| {
                p.copy_from_slice(&[1.0, 0.0, 0.0, 1.0])
            });
        let mut clone = brush(BrushTool::CloneStamp);
        clone.retouch.offset = [50.0, 0.0];
        let out = stroke(&d, clone, &[[15.0, 15.0]]);
        assert!(out.get(15, 15, 0) > 0.9 && out.get(15, 15, 1) < 0.1);
        let mut smudge = brush(BrushTool::Smudge);
        smudge.retouch.strength = 0.9;
        let out = stroke(&d, smudge, &[[65.0, 15.0], [85.0, 15.0]]);
        assert!(
            out.get(76, 15, 0) > out.get(76, 15, 1) + 0.2,
            "red was dragged along"
        );
    }

    #[test]
    fn spot_healing_and_healing_hide_a_blemish() {
        let mut d = doc();
        let id = d.layers[0].id;
        d.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::new(44, 28, 6, 6), |_, _, p| {
                p.copy_from_slice(&[1.0, 0.0, 0.0, 1.0])
            });
        let mut spot = brush(BrushTool::SpotHealing);
        spot.size = 14.0;
        let out = stroke(&d, spot, &[[47.0, 31.0]]);
        assert!(
            out.get(47, 31, 0) - out.get(47, 31, 1) < 0.15,
            "red is gone"
        );
        let mut heal = brush(BrushTool::Healing);
        heal.size = 14.0;
        heal.retouch.offset = [-24.0, 0.0];
        let out = stroke(&d, heal, &[[47.0, 31.0]]);
        assert!(out.get(47, 31, 0) - out.get(47, 31, 1) < 0.15);
    }

    #[test]
    fn selection_retouching_runs_and_undoes() {
        let mut d = doc();
        let id = d.layers[0].id;
        d.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::new(40, 24, 8, 8), |_, _, p| {
                p.copy_from_slice(&[0.9, 0.1, 0.1, 1.0])
            });
        let before = px(&d, 44, 28);
        let mut h = History::new(HistoryLimits::default());
        let mut run = |d: &mut ImageDocument, kind: &str, p: serde_json::Value| {
            let cmd = registry()
                .instantiate(&CommandRecord::new(kind, p))
                .expect(kind);
            h.execute(cmd, d).expect(kind);
        };
        run(
            &mut d,
            "image.select.shape",
            serde_json::json!({ "shape": { "type": "rect", "x": 38.0, "y": 22.0, "w": 12.0, "h": 12.0 } }),
        );
        run(&mut d, "image.retouch.fill", serde_json::json!({}));
        assert!(px(&d, 44, 28)[0] - px(&d, 44, 28)[1] < 0.15);
        run(
            &mut d,
            "image.retouch.red_eye",
            serde_json::json!({ "rect": { "x": 0, "y": 0, "w": 96, "h": 64 } }),
        );
        run(
            &mut d,
            "image.retouch.patch",
            serde_json::json!({ "dx": -30, "dy": 0 }),
        );
        run(
            &mut d,
            "image.retouch.move",
            serde_json::json!({ "dx": 20, "dy": 10 }),
        );
        assert!(d.selection.as_ref().is_some_and(|s| s.get(64, 38, 0) > 0.9));
        for _ in 0..5 {
            h.undo(&mut d).expect("undo");
        }
        assert_eq!(px(&d, 44, 28), before);
    }

    #[test]
    fn red_eye_only_touches_red() {
        assert!(redness([0.9, 0.1, 0.1]) > 0.9);
        assert_eq!(redness([0.5, 0.5, 0.5]), 0.0);
        assert_eq!(redness([0.2, 0.5, 0.9]), 0.0);
    }
}
