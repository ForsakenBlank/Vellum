//! the image editor for the app: opening and saving files, live strokes and previews, and
//! keeping each document's viewport in step with its history.
//!
//! documents live in the core document manager like every other document. this service
//! adds what only images need: it opens picture files in the background as jobs, turns a
//! document into a render graph whenever it changes, and holds what a tool shows while it
//! is in use (a stroke being drawn, a layer being dragged) outside the document, so the
//! history only ever sees finished edits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_core::app::{AppServices, SaveOutcome};
use vellum_core::command::{CommandError, CommandRecord};
use vellum_core::documents::{DocumentInfo, DocumentMeta, DocumentsChanged, RenameDocument};
use vellum_core::events::Event;
use vellum_core::files::{FileKind, Workspace, display_name};
use vellum_core::history::HistoryChanged;
use vellum_core::jobs::{JobError, JobId, JobSpec};
use vellum_core::notifications::{NewNotification, NotificationLevel};
use vellum_gpu::view::ViewSettings;
use vellum_gpu::{ColorTransform, ViewHub, ViewInfo, WorkingSpace};
use vellum_media::MediaService;
use vellum_media::picture::{self, DecodeOptions};

use crate::commands::{self, Cmd, PaintStroke};
use crate::compose::{self, Preview};
use crate::document::{DocColor, ImageDocument, Layer, LayerId};
use crate::export::ExportOptions;
use crate::paint::{self, BrushSettings, Stroke, StrokePoint};
use crate::raster::{Depth, Raster};
use crate::selection::{self, EdgeMap};
use crate::transform::{self, TransformSpec, TransformTarget};
use crate::{flatten, format};

/// the display transform for a working space, from the render settings
pub type DisplayFn = Arc<dyn Fn(WorkingSpace) -> ColorTransform + Send + Sync>;

/// sent when what a document's viewport shows has changed, so the window asks for tiles
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageChanged {
    pub document: String,
    pub width: u32,
    pub height: u32,
}

impl Event for ImageChanged {
    const NAME: &'static str = "image:changed";
}

/// what the new image dialog asks for
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewImage {
    pub title: Option<String>,
    pub width: u32,
    pub height: u32,
    pub depth: Depth,
    /// straight srgb, none for a see through background
    pub background: Option<[f32; 4]>,
}

/// what a tool shows while it is in use
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewRequest {
    /// layers dragged by the move tool, and by how much
    pub offset: Option<(Vec<LayerId>, i32, i32)>,
    /// a layer whose mask is shown on its own
    pub show_mask: Option<LayerId>,
    /// a free transform in progress
    pub transform: Option<TransformRequest>,
    /// a filter dialog's live preview
    pub filter: Option<compose::FilterPreview>,
}

/// a free transform the window is showing or committing
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransformRequest {
    pub target: TransformTarget,
    pub spec: TransformSpec,
}

struct LiveStroke {
    stroke: Stroke,
    base: Raster,
}

#[derive(Default)]
struct Live {
    stroke: Option<LiveStroke>,
    preview: Preview,
    edges: Option<Arc<EdgeMap>>,
    /// liquify or puppet warp in progress, on this layer
    warp: Option<(LayerId, crate::warp::Mesh)>,
    /// the canvas size the viewport was opened with
    size: Option<(u32, u32)>,
}

struct Inner {
    app: AppServices,
    media: MediaService,
    hub: Arc<OnceLock<ViewHub>>,
    display: DisplayFn,
    live: Mutex<HashMap<String, Live>>,
    untitled: Mutex<u32>,
}

#[derive(Clone)]
pub struct ImageService {
    inner: Arc<Inner>,
}

fn failed(err: impl std::fmt::Display) -> String {
    err.to_string()
}

impl ImageService {
    /// registers image documents with the app and starts following their changes.
    /// `hub` is filled in once the render engine has started.
    pub fn start(
        app: &AppServices,
        media: &MediaService,
        hub: Arc<OnceLock<ViewHub>>,
        display: DisplayFn,
    ) -> Self {
        app.documents.register_type::<ImageDocument>();
        let service = Self {
            inner: Arc::new(Inner {
                app: app.clone(),
                media: media.clone(),
                hub,
                display,
                live: Mutex::default(),
                untitled: Mutex::new(0),
            }),
        };
        let opener = service.clone();
        app.set_opener(
            Workspace::Image,
            Arc::new(move |path, kind, window| opener.open_file(path, kind, window)),
        );
        let saver = service.clone();
        app.set_saver(ImageDocument::TYPE_NAME, Arc::new(move |id| saver.save(id)));
        crate::fonts::set_font_folder(&app.paths.data_dir.join("fonts"));
        service.follow();
        service
    }

    fn app(&self) -> &AppServices {
        &self.inner.app
    }

    /// refreshes viewports after undo, redo and anything else that changes history, and
    /// forgets live state of closed documents
    fn follow(&self) {
        let history = self.app().bus.subscribe::<HistoryChanged>();
        let docs = self.app().bus.subscribe::<DocumentsChanged>();
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("vellum-image-follow".into())
            .spawn(move || {
                while let Some(change) = history.recv() {
                    if me.app().documents.doc_type(&change.document).ok()
                        == Some(ImageDocument::TYPE_NAME)
                    {
                        me.refresh(&change.document);
                    }
                    for list in docs.drain() {
                        let open: Vec<&str> =
                            list.documents.iter().map(|d| d.id.as_str()).collect();
                        me.inner
                            .live
                            .lock()
                            .retain(|id, _| open.contains(&id.as_str()));
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "image views will not follow undo and redo");
        }
    }

    /// opens a picture or project as an image document. the tab appears at once and the
    /// file loads in a background job.
    pub fn open_file(
        &self,
        path: &Path,
        kind: FileKind,
        window: &str,
    ) -> Result<DocumentInfo, String> {
        let mut meta = DocumentMeta::for_file(path.to_path_buf(), kind);
        meta.window = window.to_owned();
        let mut doc = ImageDocument::empty("", 1, 1, Depth::U8);
        doc.loading = Some(path.display().to_string());
        let info = self.app().documents.open(doc, meta).map_err(failed)?;
        let id = info.id.clone();
        let path = path.to_path_buf();
        let me = self.clone();
        let name = display_name(&path);
        self.app().jobs.submit(
            JobSpec::cpu(format!("Open {name}"))
                .background()
                .owned_by(id.clone())
                .about(path.display().to_string()),
            move |ctx: &vellum_core::jobs::JobContext| {
                ctx.checkpoint()?;
                match me.load(&path) {
                    Ok(doc) => {
                        // the tab may have been closed while the file loaded
                        if me.app().documents.is_open(&id) {
                            me.app()
                                .documents
                                .replace(&id, doc)
                                .map_err(JobError::fatal)?;
                            me.refresh(&id);
                        }
                    }
                    Err(err) => {
                        let _ = me.app().documents.close(&id);
                        me.app().notifications.push(
                            NewNotification::new(
                                NotificationLevel::Error,
                                format!("{name} could not be opened"),
                            )
                            .body(err),
                        );
                    }
                }
                Ok(serde_json::json!({}))
            },
        );
        Ok(info)
    }

    fn load(&self, path: &Path) -> Result<ImageDocument, String> {
        let is_project = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("vellum"));
        if is_project {
            let mut doc = format::open(path)?;
            doc.title.clear();
            return Ok(doc);
        }
        let layered = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("psb"));
        if layered {
            match crate::psd::open(path) {
                Ok(doc) => return Ok(doc),
                // modes without layers vellum can keep (cmyk, lab, bitmap) open flattened
                Err(err) => tracing::info!(%err, "opening a photoshop file flattened"),
            }
        }
        let mut pic = picture::decode(path, DecodeOptions::default(), self.inner.media.ffmpeg())
            .map_err(failed)?;
        let icc = pic.icc.take();
        if pic.frames.is_empty() {
            return Err("The picture has nothing in it to show.".into());
        }
        let image = pic.frames.swap_remove(0).image;
        let (w, h) = image.dimensions();
        let mut doc = ImageDocument::empty("", w, h, Depth::U8);
        if let Some(bytes) = icc {
            doc.color = DocColor::from_icc(bytes);
        }
        let id = doc.new_id();
        doc.layers.push(Layer::pixels(
            id,
            "Background",
            Raster::from_rgba8(w, h, image.as_raw(), Depth::U8),
        ));
        doc.active = Some(id);
        Ok(doc)
    }

    /// a new blank document
    pub fn new_document(&self, spec: &NewImage, window: &str) -> Result<DocumentInfo, String> {
        let title = match spec
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(t) => t.to_owned(),
            None => {
                let mut n = self.inner.untitled.lock();
                *n += 1;
                format!("Untitled-{n}")
            }
        };
        let (w, h) = (spec.width.clamp(1, 300_000), spec.height.clamp(1, 300_000));
        let doc = ImageDocument::new(title, w, h, spec.depth, spec.background);
        let meta = DocumentMeta {
            path: None,
            kind: Some(FileKind::Image),
            workspace: Workspace::Image,
            tool: None,
            inputs: Vec::new(),
            window: window.to_owned(),
        };
        self.app().documents.open(doc, meta).map_err(failed)
    }

    fn read<R>(&self, id: &str, f: impl FnOnce(&ImageDocument) -> R) -> Result<R, String> {
        self.app()
            .documents
            .read::<ImageDocument, R>(id, f)
            .map_err(failed)
    }

    /// `file.save`: saves to the document's `.vellum` file, or asks where to save
    pub fn save(&self, id: &str) -> Result<SaveOutcome, String> {
        let info = self.app().documents.info(id).map_err(failed)?;
        match info.path.map(PathBuf::from) {
            Some(path)
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("vellum")) =>
            {
                self.save_to(id, &path)?;
                Ok(SaveOutcome::Started)
            }
            _ => Ok(SaveOutcome::NeedsPath),
        }
    }

    /// saves to a new `.vellum` file and keeps working in it
    pub fn save_as(&self, id: &str, path: &Path) -> Result<JobId, String> {
        let path = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("vellum"))
        {
            path.to_path_buf()
        } else {
            path.with_extension("vellum")
        };
        let job = self.save_to(id, &path)?;
        Ok(job)
    }

    fn save_to(&self, id: &str, path: &Path) -> Result<JobId, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        if doc.loading.is_some() {
            return Err("The picture is still opening.".into());
        }
        let me = self.clone();
        let (id, path) = (id.to_owned(), path.to_path_buf());
        let name = display_name(&path);
        Ok(self.app().jobs.submit(
            JobSpec::io(format!("Save {name}"))
                .owned_by(id.clone())
                .about(path.display().to_string()),
            move |ctx: &vellum_core::jobs::JobContext| {
                ctx.checkpoint()?;
                let thumb = flatten::thumbnail_png(&doc, 256).ok();
                ctx.set_progress(0.2);
                format::save(&doc, &path, thumb).map_err(JobError::failed)?;
                let docs = &me.app().documents;
                if docs.is_open(&id) {
                    let moved = docs.info(&id).ok().and_then(|i| i.path)
                        != Some(path.display().to_string());
                    if moved {
                        docs.set_path(&id, path.clone()).map_err(JobError::fatal)?;
                        // a new document took its name from the file it was saved as
                        if !doc.title.is_empty() {
                            let stem = path
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let rename = CommandRecord::new(
                                RenameDocument::KIND,
                                serde_json::json!({ "title": stem }),
                            );
                            let _ = docs.execute(&id, &rename);
                        }
                    }
                    docs.mark_saved(&id).map_err(JobError::fatal)?;
                }
                Ok(serde_json::json!({ "path": path.display().to_string() }))
            },
        ))
    }

    /// exports the picture as png, jpeg, webp or avif in a background job. the result
    /// says what was done, like the quick tools' results, so the jobs panel can open it.
    pub fn export(&self, id: &str, path: &Path, options: ExportOptions) -> Result<JobId, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        if doc.loading.is_some() {
            return Err("The picture is still opening.".into());
        }
        let ext = options.format.extension();
        let path = if path.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case(ext) || (ext == "jpg" && e.eq_ignore_ascii_case("jpeg"))
        }) {
            path.to_path_buf()
        } else {
            path.with_extension(ext)
        };
        let me = self.clone();
        let name = display_name(&path);
        Ok(self.app().jobs.submit(
            JobSpec::cpu(format!("Export {name}"))
                .owned_by(id.to_owned())
                .about(path.display().to_string()),
            move |ctx: &vellum_core::jobs::JobContext| {
                ctx.checkpoint()?;
                let media = &me.inner.media;
                let hardware = media.hardware();
                let env = vellum_export::Env {
                    ffmpeg: media.ffmpeg(),
                    hardware: &hardware,
                    allow_hardware: false,
                    output_folder: None,
                };
                let margin = me.app().settings.get().export.safety_margin_percent;
                match crate::export::export(&doc, &path, &options, &env, margin, ctx) {
                    Ok(result) => Ok(serde_json::to_value(result).map_err(JobError::failed)?),
                    Err(err) => {
                        // a cancel shows as cancelled rather than as a failure
                        ctx.checkpoint()?;
                        Err(JobError::failed(err))
                    }
                }
            },
        ))
    }

    fn view_settings(&self, doc: &ImageDocument) -> ViewSettings {
        let working = doc.working();
        ViewSettings {
            precision: doc.precision(),
            working,
            display: (self.inner.display)(working),
            ..Default::default()
        }
    }

    /// opens the document's viewport, or returns the one already open
    pub fn open_view(&self, id: &str) -> Result<ViewInfo, String> {
        let hub = self.inner.hub.wait();
        let size = self.read(id, |d| (d.width, d.height, d.loading.is_some()))?;
        if size.2 {
            return Err("The picture is still opening.".into());
        }
        if let Some(info) = hub.info(id)
            && (info.width, info.height) == (size.0, size.1)
        {
            return Ok(info);
        }
        self.reopen(id, hub)
    }

    fn reopen(&self, id: &str, hub: &ViewHub) -> Result<ViewInfo, String> {
        let preview = self.preview(id);
        let (graph, node, info, settings) = self.read(id, |doc| {
            let (g, n) = compose::build(doc, &preview);
            let info = ViewInfo {
                width: doc.width,
                height: doc.height,
                profile: doc.color.name.clone(),
            };
            (g, n, info, self.view_settings(doc))
        })?;
        hub.open(id, graph, node, info.clone(), settings);
        self.inner
            .live
            .lock()
            .entry(id.to_owned())
            .or_default()
            .size = Some((info.width, info.height));
        Ok(info)
    }

    /// rebuilds every open image view from scratch, eg after the display profile changed
    pub fn reopen_views(&self) {
        let Some(hub) = self.inner.hub.get() else {
            return;
        };
        let ids: Vec<String> = self.inner.live.lock().keys().cloned().collect();
        for id in ids.iter().filter(|id| hub.is_open(id)) {
            if self.reopen(id, hub).is_ok() {
                self.app().bus.emit(ImageChanged {
                    document: id.clone(),
                    width: 0,
                    height: 0,
                });
            }
        }
    }

    fn preview(&self, id: &str) -> Preview {
        self.inner
            .live
            .lock()
            .get(id)
            .map(|l| l.preview.clone())
            .unwrap_or_default()
    }

    /// rebuilds the viewport's graph after a change and tells the window
    pub fn refresh(&self, id: &str) {
        let Some(hub) = self.inner.hub.get() else {
            return;
        };
        if !hub.is_open(id) {
            return;
        }
        let preview = self.preview(id);
        let Ok((graph, node, size)) = self.read(id, |doc| {
            let (g, n) = compose::build(doc, &preview);
            (g, n, (doc.width, doc.height))
        }) else {
            return;
        };
        let known = self.inner.live.lock().get(id).and_then(|l| l.size);
        let result = if known == Some(size) {
            hub.update(id, graph, node).map_err(failed)
        } else {
            self.reopen(id, hub).map(|_| ())
        };
        if let Err(err) = result {
            tracing::warn!(%err, id, "an image view could not be updated");
            return;
        }
        self.app().bus.emit(ImageChanged {
            document: id.to_owned(),
            width: size.0,
            height: size.1,
        });
    }

    /// runs an image command, then shows the result
    pub fn execute(&self, id: &str, record: &CommandRecord) -> Result<(), String> {
        self.app()
            .documents
            .execute(id, record)
            .map_err(|err| match err {
                vellum_core::documents::DocumentError::Command(CommandError::Failed(m)) => m,
                other => other.to_string(),
            })?;
        self.refresh(id);
        Ok(())
    }

    /// starts a brush, pencil or eraser stroke on whatever the document is set to paint
    pub fn begin_stroke(&self, id: &str, settings: BrushSettings) -> Result<(), String> {
        let (stroke, base) = self.read(id, |doc| {
            if doc.loading.is_some() {
                return Err("The picture is still opening.".to_owned());
            }
            let stroke = Stroke::new(doc, settings)?;
            let base = paint::raster_for(doc, &stroke.part)?;
            Ok((stroke, base))
        })??;
        let mut live = self.inner.live.lock();
        let l = live.entry(id.to_owned()).or_default();
        l.preview.replace = Some((stroke.part.clone(), base.clone()));
        l.stroke = Some(LiveStroke { stroke, base });
        Ok(())
    }

    /// paints more of the stroke and shows it
    pub fn stroke(&self, id: &str, points: &[StrokePoint]) -> Result<(), String> {
        {
            let mut live = self.inner.live.lock();
            let l = live.get_mut(id).ok_or("No stroke is being drawn.")?;
            let s = l.stroke.as_mut().ok_or("No stroke is being drawn.")?;
            s.stroke.add(points);
            l.preview.replace = Some((s.stroke.part.clone(), s.stroke.result().clone()));
        }
        self.refresh(id);
        Ok(())
    }

    /// finishes the stroke as one undoable step
    pub fn end_stroke(&self, id: &str) -> Result<(), String> {
        let live = {
            let mut live = self.inner.live.lock();
            let l = live.get_mut(id).ok_or("No stroke is being drawn.")?;
            l.preview.replace = None;
            l.stroke.take()
        };
        let Some(mut live) = live else {
            return Err("No stroke is being drawn.".into());
        };
        live.stroke.finish();
        let result = if live.stroke.points().is_empty() {
            Ok(())
        } else {
            let cmd = PaintStroke::from_stroke(&live.stroke, live.base);
            self.app()
                .documents
                .execute_command::<ImageDocument>(id, Box::new(Cmd::new(cmd)))
                .map_err(failed)
        };
        self.refresh(id);
        result
    }

    pub fn cancel_stroke(&self, id: &str) {
        if let Some(l) = self.inner.live.lock().get_mut(id) {
            l.stroke = None;
            l.preview.replace = None;
        }
        self.refresh(id);
    }

    /// shows a tool's temporary change, eg a layer being dragged
    pub fn set_preview(&self, id: &str, request: PreviewRequest) {
        let transform = request.transform.and_then(|t| {
            let plan = self.read(id, |d| transform::plan(d, t.target)).ok()?.ok()?;
            Some(compose::TransformPreview { spec: t.spec, plan })
        });
        {
            let mut live = self.inner.live.lock();
            let l = live.entry(id.to_owned()).or_default();
            l.preview.offset = request.offset;
            l.preview.show_mask = request.show_mask;
            l.preview.transform = transform;
            l.preview.filter = request.filter;
        }
        self.refresh(id);
    }

    /// the box free transform starts from, x y w h, or an error saying why it cannot
    pub fn transform_box(&self, id: &str, target: TransformTarget) -> Result<[i32; 4], String> {
        let b = self.read(id, |d| transform::start_box(d, target))??;
        if b.is_empty() {
            return Err(
                "There are no pixels to transform here. Pick a layer with something on it.".into(),
            );
        }
        Ok([b.x, b.y, b.w as i32, b.h as i32])
    }

    /// commits a free transform as one undoable step. the pixels are worked out before
    /// the document is locked, so other documents do not wait for a big transform.
    pub fn commit_transform(&self, id: &str, request: TransformRequest) -> Result<(), String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let prepared = transform::prepare(&doc, request.target, &request.spec);
        {
            let mut live = self.inner.live.lock();
            if let Some(l) = live.get_mut(id) {
                l.preview.transform = None;
            }
        }
        let result = prepared.and_then(|parts| {
            let edit = transform::Transform {
                spec: request.spec,
                target: request.target,
                prepared: Some(parts),
            };
            self.app()
                .documents
                .execute_command::<ImageDocument>(id, Box::new(Cmd::new(edit)))
                .map_err(|err| match err {
                    vellum_core::documents::DocumentError::Command(CommandError::Failed(m)) => m,
                    other => other.to_string(),
                })
        });
        self.refresh(id);
        result
    }

    /// the selection outline for marching ants, at a mip level that suits the zoom
    pub fn outline(&self, id: &str, level: u32) -> Result<Vec<Vec<[f32; 2]>>, String> {
        let sel = self.read(id, |d| {
            if d.quick_mask {
                None
            } else {
                d.selection.clone()
            }
        })?;
        Ok(sel
            .map(|s| selection::outline_at_level(&s, level.min(8)))
            .unwrap_or_default())
    }

    /// starts a magnetic lasso drag by finding the picture's edges
    pub fn begin_magnetic(&self, id: &str, sample_all: bool, contrast: f32) -> Result<(), String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let edges = Arc::new(commands::edges_for(&doc, sample_all, contrast)?);
        self.inner
            .live
            .lock()
            .entry(id.to_owned())
            .or_default()
            .edges = Some(edges);
        Ok(())
    }

    /// the magnetic lasso's path from the last anchor towards the pointer
    pub fn magnetic(
        &self,
        id: &str,
        from: (i32, i32),
        to: (i32, i32),
        width: i32,
    ) -> Result<Vec<[i32; 2]>, String> {
        let edges = self
            .inner
            .live
            .lock()
            .get(id)
            .and_then(|l| l.edges.clone())
            .ok_or("Start the magnetic lasso first.")?;
        Ok(selection::live_wire(&edges, from, to, width)
            .into_iter()
            .map(|(x, y)| [x, y])
            .collect())
    }

    pub fn end_magnetic(&self, id: &str) {
        if let Some(l) = self.inner.live.lock().get_mut(id) {
            l.edges = None;
        }
    }

    /// the eyedropper's colour, straight srgb with alpha
    pub fn pick_color(
        &self,
        id: &str,
        x: i32,
        y: i32,
        size: u32,
        all: bool,
    ) -> Result<[f32; 4], String> {
        let doc = self.read(id, ImageDocument::clone)?;
        commands::pick_color(&doc, x, y, size.clamp(1, 101), all)
    }

    /// small pictures for the layers panel and the navigator, packed as
    /// `(u32 layer id or 0 for the whole picture, u8 is mask, u32 width, u32 height, rgba)`
    /// one after another, little endian
    pub fn thumbnails(
        &self,
        id: &str,
        layers: &[(u32, bool)],
        size: u32,
    ) -> Result<Vec<u8>, String> {
        let doc = self.read(id, ImageDocument::clone)?;
        let mut out = Vec::new();
        for &(layer, mask) in layers {
            let only = (layer != 0).then_some(LayerId(layer));
            let Ok((w, h, px)) = flatten::thumbnail(&doc, only, mask, size.clamp(8, 512)) else {
                continue;
            };
            out.extend_from_slice(&layer.to_le_bytes());
            out.push(u8::from(mask));
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
            out.extend_from_slice(&px);
        }
        Ok(out)
    }
}

mod tools;
pub use tools::{FontPreview, NodeCatalogue};

impl ImageDocument {
    /// the document type's name, as [`vellum_core::documents::DocumentModel::TYPE`]
    pub const TYPE_NAME: &'static str = "image";
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_core::app::AppPaths;

    fn start() -> (tempfile::TempDir, AppServices, ImageService) {
        let dir = tempfile::tempdir().unwrap();
        let app = AppServices::start(AppPaths {
            config_dir: dir.path().join("config"),
            data_dir: dir.path().join("data"),
        })
        .unwrap();
        let media = MediaService::start(&app, &dir.path().join("cache"));
        let hub = Arc::new(OnceLock::new());
        let _ = hub.set(ViewHub::new(Arc::new(vellum_gpu::Engine::cpu_only())));
        let display: DisplayFn =
            Arc::new(|w| ColorTransform::between(w.space(), vellum_gpu::ColorSpace::SRGB));
        let service = ImageService::start(&app, &media, hub, display);
        (dir, app, service)
    }

    #[test]
    fn a_live_stroke_becomes_one_undoable_step() {
        let (_dir, app, service) = start();
        let info = service
            .new_document(
                &NewImage {
                    title: None,
                    width: 300,
                    height: 200,
                    depth: Depth::U8,
                    background: Some([1.0; 4]),
                },
                "main",
            )
            .unwrap();
        assert_eq!(info.title, "Untitled-1");
        service.open_view(&info.id).unwrap();
        service
            .begin_stroke(
                &info.id,
                BrushSettings {
                    smoothing: 0.0,
                    ..Default::default()
                },
            )
            .unwrap();
        let pts: Vec<StrokePoint> = (0..10)
            .map(|i| StrokePoint {
                x: 20.0 + i as f32 * 20.0,
                y: 100.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
            })
            .collect();
        service.stroke(&info.id, &pts).unwrap();
        // nothing reaches the document until the stroke ends
        let untouched = service
            .read(&info.id, |d| d.layers[0].raster().unwrap().get(100, 100, 0))
            .unwrap();
        assert_eq!(untouched, 1.0);
        service.end_stroke(&info.id).unwrap();
        let painted = service
            .read(&info.id, |d| d.layers[0].raster().unwrap().get(100, 100, 0))
            .unwrap();
        assert!(painted < 0.1);
        assert!(app.documents.undo(&info.id).unwrap());
        let undone = service
            .read(&info.id, |d| d.layers[0].raster().unwrap().get(100, 100, 0))
            .unwrap();
        assert_eq!(undone, 1.0);
    }

    fn wait(app: &AppServices, job: JobId) -> vellum_core::jobs::JobSnapshot {
        for _ in 0..3000 {
            if let Some(j) = app.jobs.snapshot(job).filter(|j| j.state.is_finished()) {
                return j;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the job did not finish");
    }

    #[test]
    fn export_writes_every_format_as_a_job() {
        let (dir, app, service) = start();
        let info = service
            .new_document(
                &NewImage {
                    title: Some("art".into()),
                    width: 64,
                    height: 48,
                    depth: Depth::U8,
                    background: Some([0.2, 0.6, 0.9, 1.0]),
                },
                "main",
            )
            .unwrap();
        let has_ffmpeg = vellum_media::ffmpeg::Ffmpeg::locate().is_ok();
        for format in [
            crate::export::ExportFormat::Png,
            crate::export::ExportFormat::Jpeg,
            crate::export::ExportFormat::Webp,
            crate::export::ExportFormat::Avif,
        ] {
            let needs_ffmpeg = matches!(
                format,
                crate::export::ExportFormat::Webp | crate::export::ExportFormat::Avif
            );
            if needs_ffmpeg && !has_ffmpeg {
                assert!(
                    std::env::var_os("VELLUM_REQUIRE_FFMPEG").is_none(),
                    "ffmpeg is missing"
                );
                continue;
            }
            // the extension follows the format
            let path = dir.path().join("out.bin");
            let options = ExportOptions {
                format,
                ..ExportOptions::default()
            };
            let job = service.export(&info.id, &path, options).unwrap();
            let done = wait(&app, job);
            assert!(
                matches!(done.state, vellum_core::jobs::JobState::Completed),
                "{format:?}: {:?}",
                done.state
            );
            let written = dir.path().join(format!("out.{}", format.extension()));
            let probe = image::ImageReader::open(&written).and_then(|r| r.with_guessed_format());
            assert!(written.exists() && probe.is_ok(), "{format:?}");
        }
    }

    #[test]
    fn pictures_open_in_the_background_and_save_as_projects() {
        let (dir, app, service) = start();
        let png = dir.path().join("photo.png");
        image::RgbaImage::from_pixel(40, 30, image::Rgba([10, 200, 30, 255]))
            .save(&png)
            .unwrap();
        let opened = app.open_files(std::slice::from_ref(&png), None, "main");
        let id = opened.opened[0].id.clone();
        assert_eq!(opened.opened[0].doc_type, "image");
        for _ in 0..200 {
            if service.read(&id, |d| d.loading.is_none()).unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            service.read(&id, |d| (d.width, d.height)).unwrap(),
            (40, 30)
        );
        assert_eq!(service.save(&id).unwrap(), SaveOutcome::NeedsPath);
        let project = dir.path().join("photo.vellum");
        let job = service.save_as(&id, &project).unwrap();
        for _ in 0..300 {
            if app
                .jobs
                .snapshot(job)
                .is_some_and(|j| j.state.is_finished())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(project.exists());
        let info = app.documents.info(&id).unwrap();
        assert_eq!(
            info.path.as_deref(),
            Some(project.display().to_string().as_str())
        );
        assert!(!info.dirty);
        let back = format::open(&project).unwrap();
        assert!((back.layers[0].raster().unwrap().get(5, 5, 1) - 200.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn phase_six_calls_work_through_the_service() {
        let (dir, app, service) = start();
        let info = service
            .new_document(
                &NewImage {
                    title: Some("six".into()),
                    width: 96,
                    height: 64,
                    depth: Depth::U8,
                    background: Some([0.4, 0.6, 0.9, 1.0]),
                },
                "main",
            )
            .unwrap();
        let id = info.id.clone();
        service.open_view(&id).unwrap();

        let cat = service.catalogue();
        assert!(cat.nodes.iter().any(|n| n.id == "effect.camera_raw"));
        assert!(cat.filters.len() > 25 && !cat.presets.is_empty() && !cat.styles.is_empty());

        let thumbs = service
            .filter_thumbnails(
                &id,
                &[tools::ThumbnailRequest {
                    entry: "stylize.emboss".into(),
                    params: Default::default(),
                }],
                48,
            )
            .unwrap();
        let w = u32::from_le_bytes(thumbs[0..4].try_into().unwrap());
        let h = u32::from_le_bytes(thumbs[4..8].try_into().unwrap());
        assert_eq!(thumbs.len(), 8 + (w * h * 4) as usize);

        // a filter shown live, then dropped, changes nothing in the document
        let layer = service.read(&id, |d| d.layers[0].id).unwrap();
        service.set_preview(
            &id,
            PreviewRequest {
                filter: Some(compose::FilterPreview {
                    layer,
                    kind: "effect.invert".into(),
                    params: Default::default(),
                }),
                ..Default::default()
            },
        );
        service.set_preview(&id, PreviewRequest::default());

        // liquify is one undo step
        service.warp_begin(&id, None).unwrap();
        service
            .liquify(
                &id,
                crate::warp::LiquifyBrush {
                    size: 40.0,
                    pressure: 1.0,
                    ..Default::default()
                },
                &[[30.0, 30.0], [50.0, 30.0], [70.0, 30.0]],
            )
            .unwrap();
        service.warp_end(&id, true, "Liquify").unwrap();
        let steps = app
            .documents
            .history(&id)
            .map(|h| h.entries.len())
            .unwrap_or(0);
        assert!(steps >= 2, "the warp is in the history");
        service.warp_begin(&id, None).unwrap();
        service
            .puppet(
                &id,
                &[
                    crate::warp::Pin {
                        from: [10.0, 10.0],
                        to: [10.0, 10.0],
                    },
                    crate::warp::Pin {
                        from: [80.0, 50.0],
                        to: [85.0, 40.0],
                    },
                ],
            )
            .unwrap();
        service.warp_end(&id, false, "Puppet warp").unwrap();

        let web = service
            .web_preview(
                &id,
                &crate::outputs::WebOptions::default(),
                vellum_gpu::IRect::EMPTY,
            )
            .unwrap();
        assert!(web.bytes > 0 && web.optimized.len() == web.original.len());

        let psd = dir.path().join("six.psd");
        let job = service.save_psd(&id, &psd).unwrap();
        assert_eq!(
            wait(&app, job).state,
            vellum_core::jobs::JobState::Completed
        );
        assert!(crate::psd::open(&psd).is_ok());
    }
}
