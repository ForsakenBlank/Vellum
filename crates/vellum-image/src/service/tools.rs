//! the image service's phase 6 calls: what the adjustment, filter and style panels list,
//! filter gallery thumbnails, liquify and puppet warp sessions, fonts and google fonts,
//! placing files as smart objects, and the exports (photoshop files, layers, artboards,
//! save for web).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_core::jobs::{JobError, JobId, JobSpec};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::graph::NodeInfo;
use vellum_gpu::{BackendChoice, IRect, ParamValue, Precision};
use vellum_media::Control;

use super::{ImageService, failed};
use crate::commands::Cmd;
use crate::compose::{self, Preview, registry};
use crate::document::{ImageDocument, LayerId};
use crate::warp::{LiquifyBrush, Mesh, MeshWarp, Pin, spacing_for};
use crate::{adjustments, fonts, outputs, smart, styles, vector};

/// everything the adjustment, filter and style panels need to build their controls
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeCatalogue {
    /// every render node's parameters, so controls are built from the list
    pub nodes: Vec<NodeInfo>,
    /// the adjustments panel, in order: node id and name
    pub adjustments: Vec<(String, String)>,
    pub presets: Vec<adjustments::Preset>,
    pub looks: Vec<String>,
    pub filters: Vec<smart::FilterEntry>,
    pub styles: Vec<styles::StylePreset>,
    pub patterns: Vec<crate::fills::Pattern>,
    pub shapes: Vec<String>,
}

/// a font family drawn in its own face for the font browser: straight rgba8
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontPreview {
    pub family: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// a filter to show as a gallery thumbnail
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThumbnailRequest {
    /// the gallery entry
    pub entry: String,
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
}

fn job_env<R>(media: &vellum_media::MediaService, f: impl FnOnce(&vellum_export::Env) -> R) -> R {
    let hardware = media.hardware();
    let env = vellum_export::Env {
        ffmpeg: media.ffmpeg(),
        hardware: &hardware,
        allow_hardware: false,
        output_folder: None,
    };
    f(&env)
}

impl ImageService {
    pub fn catalogue(&self) -> NodeCatalogue {
        NodeCatalogue {
            nodes: registry().infos().into_iter().copied().collect(),
            adjustments: adjustments::KINDS
                .iter()
                .map(|(k, t)| ((*k).to_owned(), (*t).to_owned()))
                .collect(),
            presets: adjustments::presets(),
            looks: adjustments::LOOKS.iter().map(|s| (*s).to_owned()).collect(),
            filters: smart::gallery(),
            styles: styles::presets(),
            patterns: crate::fills::patterns(),
            shapes: vector::CUSTOM_SHAPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }

    /// small pictures of the document with each filter on it, for the filter gallery:
    /// width, height and rgba8 for each, one after another (little endian sizes)
    pub fn filter_thumbnails(
        &self,
        id: &str,
        filters: &[ThumbnailRequest],
        size: u32,
    ) -> Result<Vec<u8>, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let canvas = doc.canvas();
        let mut level = 0;
        while level < 16
            && canvas.at_level(level).w.max(canvas.at_level(level).h) > size.clamp(16, 512)
        {
            level += 1;
        }
        let rect = canvas.at_level(level);
        let anchor = doc
            .active_layer()
            .map(|l| l.bounds())
            .filter(|b| !b.is_empty())
            .unwrap_or(canvas);
        let mut out = Vec::new();
        for f in filters {
            let Some(entry) = smart::gallery_entry(&f.entry) else {
                continue;
            };
            let (mut g, top) = compose::build(&doc, &Preview::default());
            let mut params = entry.params.clone();
            params.extend(f.params.clone());
            let Some(node) = smart::filter_node(&entry.kind, &params, anchor, canvas, registry())
            else {
                continue;
            };
            let top = g.add(node.input(0, top));
            let req = RenderRequest::new(top, rect)
                .level(level)
                .working(doc.working())
                .precision(Precision::F32)
                .backend(BackendChoice::Cpu);
            let img = crate::flatten::engine()
                .render_image(&g, &req)
                .map_err(|err| format!("A preview could not be drawn: {err}"))?;
            out.extend_from_slice(&rect.w.to_le_bytes());
            out.extend_from_slice(&rect.h.to_le_bytes());
            let linear = doc.depth == crate::raster::Depth::F32;
            for p in &img.data {
                let a = p[3].clamp(0.0, 1.0);
                let c = |v: f32| {
                    let s = if a > 0.0 { v / a } else { 0.0 };
                    let s = if linear {
                        vellum_gpu::Transfer::Srgb.encode(s)
                    } else {
                        s
                    };
                    (s.clamp(0.0, 1.0) * 255.0).round() as u8
                };
                out.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8]);
            }
        }
        Ok(out)
    }

    /// starts liquify or puppet warp on a layer
    pub fn warp_begin(&self, id: &str, layer: Option<LayerId>) -> Result<LayerId, String> {
        let (lid, area) = self.read(id, |d| {
            let lid = layer.or(d.active).ok_or("Pick a layer to warp first.")?;
            let l = d.layer(lid).ok_or("That layer is gone.")?;
            l.paintable()?;
            let area = l.bounds().union(&d.canvas());
            Ok::<_, String>((lid, area))
        })??;
        let mesh = Mesh::identity(area, spacing_for(area));
        let mut live = self.inner.live.lock();
        let l = live.entry(id.to_owned()).or_default();
        l.preview.mesh = Some((lid, Arc::new(mesh.clone())));
        l.warp = Some((lid, mesh));
        Ok(lid)
    }

    /// one liquify brush move: the brush at each point, moving from the one before
    pub fn liquify(
        &self,
        id: &str,
        brush: LiquifyBrush,
        points: &[[f32; 2]],
    ) -> Result<(), String> {
        {
            let mut live = self.inner.live.lock();
            let l = live.get_mut(id).ok_or("Liquify is not open.")?;
            let (lid, mesh) = l.warp.as_mut().ok_or("Liquify is not open.")?;
            for w in points.windows(2) {
                mesh.liquify(&brush, w[1], [w[1][0] - w[0][0], w[1][1] - w[0][1]]);
            }
            if points.len() == 1 {
                mesh.liquify(&brush, points[0], [0.0, 0.0]);
            }
            l.preview.mesh = Some((*lid, Arc::new(mesh.clone())));
        }
        self.refresh(id);
        Ok(())
    }

    /// puppet warp: places the mesh for these pins
    pub fn puppet(&self, id: &str, pins: &[Pin]) -> Result<(), String> {
        {
            let mut live = self.inner.live.lock();
            let l = live.get_mut(id).ok_or("Puppet warp is not open.")?;
            let (lid, mesh) = l.warp.as_mut().ok_or("Puppet warp is not open.")?;
            mesh.puppet(pins);
            l.preview.mesh = Some((*lid, Arc::new(mesh.clone())));
        }
        self.refresh(id);
        Ok(())
    }

    /// ends liquify or puppet warp, keeping the result as one undoable step or not
    pub fn warp_end(&self, id: &str, keep: bool, title: &str) -> Result<(), String> {
        let warp = {
            let mut live = self.inner.live.lock();
            let l = live.get_mut(id).ok_or("Nothing is being warped.")?;
            l.preview.mesh = None;
            l.warp.take()
        };
        let result = match warp {
            Some((lid, mesh)) if keep && !mesh.is_identity() => {
                let cmd = MeshWarp {
                    id: Some(lid),
                    title: title.to_owned(),
                    mesh,
                };
                self.app()
                    .documents
                    .execute_command::<ImageDocument>(id, Box::new(Cmd::new(cmd)))
                    .map_err(failed)
            }
            _ => Ok(()),
        };
        self.refresh(id);
        result
    }

    /// every font family installed or downloaded
    pub fn fonts(&self) -> Vec<fonts::FontFamily> {
        fonts::families()
    }

    /// families drawn in their own faces, for the font browser
    pub fn font_previews(&self, families: &[String], text: &str, size: f32) -> Vec<FontPreview> {
        families
            .iter()
            .filter_map(|family| {
                let mut t = crate::text::TextLayer::point(text, 2.0, size, size);
                t.font = fonts::FontRef {
                    family: family.clone(),
                    ..Default::default()
                };
                t.color = [1.0, 1.0, 1.0, 1.0];
                let clip = IRect::new(
                    0,
                    0,
                    (size * text.chars().count() as f32).ceil() as u32 + 8,
                    (size * 1.6).ceil() as u32,
                );
                let drawn = crate::text::draw(&t, crate::raster::Depth::U8, clip, &|c| c);
                if drawn.missing_font {
                    return None;
                }
                let r = drawn.bounds.union(&IRect::new(0, 0, 1, clip.h));
                let w = r.right().max(1) as u32;
                let h = clip.h;
                let px = drawn.pixels.read(IRect::new(0, 0, w, h));
                let pixels = px
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| p.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
                    .collect();
                Some(FontPreview {
                    family: family.clone(),
                    width: w,
                    height: h,
                    pixels,
                })
            })
            .collect()
    }

    /// the google fonts catalogue, cached for a week
    pub fn google_fonts(&self) -> Result<Vec<fonts::GoogleFont>, String> {
        let cache = self
            .app()
            .paths
            .data_dir
            .join("fonts")
            .join("google-fonts.json");
        fonts::google_catalogue(&cache)
    }

    /// downloads a google font as a job; text layers waiting for it redraw when it lands
    pub fn download_font(&self, font: fonts::GoogleFont) -> Result<JobId, String> {
        let me = self.clone();
        let family = font.family.clone();
        Ok(self.app().jobs.submit(
            JobSpec::io(format!("Download {family}")).about(family.clone()),
            move |ctx: &vellum_core::jobs::JobContext| {
                let files = fonts::download_google(&font, |done, total| {
                    ctx.progress(done as f32 / total.max(1) as f32);
                    ctx.checkpoint().is_ok()
                })
                .map_err(JobError::failed)?;
                // documents that were waiting for this family draw their text again
                for doc in me.app().documents.list() {
                    let missing = me
                        .read(&doc.id, crate::vector_ops::missing_fonts)
                        .unwrap_or_default();
                    if missing.iter().any(|f| f.eq_ignore_ascii_case(&family)) {
                        let record = vellum_core::command::CommandRecord::new(
                            "image.text.refresh",
                            serde_json::json!({}),
                        );
                        let _ = me.execute(&doc.id, &record);
                    }
                }
                Ok(serde_json::json!({ "files": files.len() }))
            },
        ))
    }

    /// fonts a document uses that are not installed
    pub fn missing_fonts(&self, id: &str) -> Result<Vec<String>, String> {
        self.read(id, crate::vector_ops::missing_fonts)
    }

    /// places a picture file as a smart object, reading it in the background
    pub fn place(&self, id: &str, path: &Path, linked: bool) -> Result<JobId, String> {
        let depth = self.read(id, |d| d.depth)?;
        let me = self.clone();
        let doc_id = id.to_owned();
        let path = path.to_path_buf();
        let name = vellum_core::files::display_name(&path);
        Ok(self.app().jobs.submit(
            JobSpec::io(format!("Place {name}")).owned_by(id.to_owned()),
            move |ctx: &vellum_core::jobs::JobContext| {
                ctx.checkpoint()?;
                let p = path.to_string_lossy().into_owned();
                let pixels = crate::layer_ops::read_picture(&p, depth).map_err(JobError::failed)?;
                let cmd = crate::layer_ops::PlaceSmart {
                    path: Some(p),
                    name: None,
                    linked,
                    pixels: Some(pixels),
                };
                me.app()
                    .documents
                    .execute_command::<ImageDocument>(&doc_id, Box::new(Cmd::new(cmd)))
                    .map_err(JobError::failed)?;
                me.refresh(&doc_id);
                Ok(serde_json::Value::Null)
            },
        ))
    }

    /// saves a photoshop file (psb when the path says so) as a job
    pub fn save_psd(&self, id: &str, path: &Path) -> Result<JobId, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let path = path.to_path_buf();
        let name = vellum_core::files::display_name(&path);
        Ok(self.app().jobs.submit(
            JobSpec::io(format!("Save {name}")).owned_by(id.to_owned()),
            move |ctx: &vellum_core::jobs::JobContext| {
                ctx.checkpoint()?;
                crate::psd_write::save(&doc, &path).map_err(JobError::failed)?;
                Ok(serde_json::json!({ "path": path.display().to_string() }))
            },
        ))
    }

    /// writes each layer to its own file as a job
    pub fn export_layers(
        &self,
        id: &str,
        dir: &Path,
        how: outputs::LayerExport,
    ) -> Result<JobId, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let me = self.clone();
        let dir = dir.to_path_buf();
        Ok(self.app().jobs.submit(
            JobSpec::cpu("Export layers to files").owned_by(id.to_owned()),
            move |ctx: &vellum_core::jobs::JobContext| {
                let margin = me.app().settings.get().export.safety_margin_percent;
                let files = job_env(&me.inner.media, |env| {
                    outputs::export_layers(&doc, &dir, &how, env, margin, ctx, |p| ctx.progress(p))
                })
                .map_err(JobError::failed)?;
                Ok(serde_json::json!({ "folder": dir.display().to_string(), "files": files }))
            },
        ))
    }

    /// writes each artboard to its own file as a job
    pub fn export_artboards(
        &self,
        id: &str,
        dir: &Path,
        options: crate::export::ExportOptions,
    ) -> Result<JobId, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let me = self.clone();
        let dir: PathBuf = dir.to_path_buf();
        Ok(self.app().jobs.submit(
            JobSpec::cpu("Export artboards").owned_by(id.to_owned()),
            move |ctx: &vellum_core::jobs::JobContext| {
                let margin = me.app().settings.get().export.safety_margin_percent;
                let files = job_env(&me.inner.media, |env| {
                    outputs::export_artboards(&doc, &dir, &options, env, margin, ctx, |p| {
                        ctx.progress(p)
                    })
                })
                .map_err(JobError::failed)?;
                Ok(serde_json::json!({ "folder": dir.display().to_string(), "files": files }))
            },
        ))
    }

    /// save for web: the encoded result next to the original, with its size
    pub fn web_preview(
        &self,
        id: &str,
        options: &outputs::WebOptions,
        view: IRect,
    ) -> Result<outputs::WebPreview, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        struct Go;
        impl vellum_media::Control for Go {
            fn checkpoint(&self) -> Result<(), vellum_media::MediaError> {
                Ok(())
            }
            fn progress(&self, _f: f32) {}
        }
        job_env(&self.inner.media, |env| {
            outputs::web_preview(&doc, options, view, env, &Go)
        })
    }
}
