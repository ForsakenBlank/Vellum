//! every open viewport in the app, keyed by an id the window chooses (the document id).
//! the desktop shell turns ipc calls into calls here, so the logic is tested without a
//! window.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use ts_rs::TS;

use crate::color::{ColorSpace, ColorTransform, IccProfile};
use crate::engine::Engine;
use crate::graph::{Graph, Node, NodeId, RenderError};
use crate::image::{AlphaMode, SourcePixels};
use crate::nodes;
use crate::rect::IRect;
use crate::tiles::{SourceColor, TiledImage};
use crate::view::{Picture, TileId, ViewSettings, ViewState, Viewport, pack};

/// what a window needs to know about a picture before it asks for tiles
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewInfo {
    pub width: u32,
    pub height: u32,
    /// the colour profile the picture came with, if any
    pub profile: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("that picture is not open")]
    NotOpen,
    #[error(transparent)]
    Render(#[from] RenderError),
}

struct Shown {
    graph: Arc<Graph>,
    node: NodeId,
    revision: u64,
    viewport: Viewport,
    settings: ViewSettings,
    info: ViewInfo,
}

#[derive(Clone)]
pub struct ViewHub {
    engine: Arc<Engine>,
    shown: Arc<Mutex<HashMap<String, Arc<Mutex<Shown>>>>>,
}

impl ViewHub {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            shown: Arc::default(),
        }
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    /// shows `node` of `graph` under `id`, replacing what was there
    pub fn open(
        &self,
        id: &str,
        graph: Graph,
        node: NodeId,
        info: ViewInfo,
        settings: ViewSettings,
    ) {
        let doc = IRect::of_size(info.width, info.height);
        let revision = self
            .shown
            .lock()
            .get(id)
            .map_or(1, |s| s.lock().revision + 1);
        self.shown.lock().insert(
            id.to_owned(),
            Arc::new(Mutex::new(Shown {
                graph: Arc::new(graph),
                node,
                revision,
                viewport: Viewport::new(doc),
                settings,
                info,
            })),
        );
    }

    /// a new graph for an open picture, eg after an edit. tiles whose content did not
    /// change are not sent again.
    pub fn update(&self, id: &str, graph: Graph, node: NodeId) -> Result<(), HubError> {
        let shown = self.get(id)?;
        let mut s = shown.lock();
        s.graph = Arc::new(graph);
        s.node = node;
        s.revision += 1;
        Ok(())
    }

    /// a new display transform for every open picture, eg when the display profile
    /// changes in settings. tiles the windows hold are replaced as they are asked for.
    pub fn set_display(&self, display: ColorTransform) {
        for shown in self.shown.lock().values() {
            let mut s = shown.lock();
            s.settings.display = display.clone();
            s.revision += 1;
        }
    }

    /// shows a picture made of straight rgba8 pixels, with the colour profile it came
    /// with. a profile that cannot be read falls back to srgb and says so in the info.
    pub fn open_rgba8(
        &self,
        id: &str,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        icc: Option<&[u8]>,
        settings: ViewSettings,
    ) -> ViewInfo {
        let profile = icc.and_then(|bytes| match IccProfile::from_bytes(bytes) {
            Ok(p) => Some(p),
            Err(err) => {
                tracing::warn!(%err, "showing a picture as srgb");
                None
            }
        });
        let info = ViewInfo {
            width,
            height,
            profile: profile.as_ref().map(|p| {
                p.description
                    .clone()
                    .unwrap_or_else(|| "Embedded profile".into())
            }),
        };
        let color = match profile {
            Some(p) => SourceColor::Icc(p),
            None => SourceColor::Space(ColorSpace::SRGB),
        };
        let px = SourcePixels::rgba8(IRect::of_size(width, height), AlphaMode::Straight, pixels);
        let mut graph = Graph::new();
        let node = graph
            .add(Node::of(nodes::ImageNode).source(Arc::new(TiledImage::from_pixels(&px, color))));
        self.open(id, graph, node, info.clone(), settings);
        info
    }

    pub fn info(&self, id: &str) -> Option<ViewInfo> {
        self.shown.lock().get(id).map(|s| s.lock().info.clone())
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.shown.lock().contains_key(id)
    }

    pub fn close(&self, id: &str) {
        self.shown.lock().remove(id);
    }

    fn get(&self, id: &str) -> Result<Arc<Mutex<Shown>>, HubError> {
        self.shown.lock().get(id).cloned().ok_or(HubError::NotOpen)
    }

    /// the tiles the window is missing for `view`, packed as described in
    /// [`crate::view::pack`]. `evicted` lists tiles the window dropped since it last asked;
    /// `reset` says it holds nothing at all.
    pub fn frame(
        &self,
        id: &str,
        view: &ViewState,
        evicted: &[TileId],
        reset: bool,
        budget: Duration,
    ) -> Result<Vec<u8>, HubError> {
        let shown = self.get(id)?;
        let mut s = shown.lock();
        if reset {
            s.viewport.reset();
        }
        s.viewport.forget(evicted);
        let Shown {
            graph,
            node,
            revision,
            viewport,
            settings,
            ..
        } = &mut *s;
        let picture = Picture {
            graph,
            node: *node,
            revision: *revision,
        };
        let update = viewport.frame(&self.engine, &picture, view, settings, budget)?;
        Ok(pack(&update))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Node;
    use crate::nodes;
    use crate::tiles::ProceduralSource;

    fn picture() -> (Graph, NodeId) {
        let mut g = Graph::new();
        let src = Arc::new(ProceduralSource::new(600, 400, |x, _| {
            [x / 600.0, 0.5, 0.5, 1.0]
        }));
        let n = g.add(Node::of(nodes::ImageNode).source(src));
        (g, n)
    }

    fn view() -> ViewState {
        ViewState {
            center_x: 300.0,
            center_y: 200.0,
            zoom: 1.0,
            rotation: 0.0,
            flip_x: false,
            width: 640,
            height: 480,
        }
    }

    fn tile_count(frame: &[u8]) -> u32 {
        u32::from_le_bytes(frame[4..8].try_into().unwrap_or_default())
    }

    #[test]
    fn frames_follow_the_picture_and_the_window() {
        let hub = ViewHub::new(Arc::new(Engine::cpu_only()));
        let (g, n) = picture();
        let info = ViewInfo {
            width: 600,
            height: 400,
            profile: None,
        };
        hub.open("doc-1", g, n, info.clone(), ViewSettings::default());
        assert_eq!(hub.info("doc-1"), Some(info));
        let long = Duration::from_secs(30);
        let first = hub.frame("doc-1", &view(), &[], false, long).unwrap();
        assert_eq!(tile_count(&first), 6);
        assert_eq!(
            tile_count(&hub.frame("doc-1", &view(), &[], false, long).unwrap()),
            0
        );
        // a window that lost its tiles gets them all again
        assert_eq!(
            tile_count(&hub.frame("doc-1", &view(), &[], true, long).unwrap()),
            6
        );
        // an edit resends only what changed: here, everything
        let (mut g2, n2) = picture();
        let lv = g2.add(Node::of(nodes::Levels).param("gamma", 2.0).input(0, n2));
        hub.update("doc-1", g2, lv).unwrap();
        assert_eq!(
            tile_count(&hub.frame("doc-1", &view(), &[], false, long).unwrap()),
            6
        );
        // a new display transform makes every held tile stale
        hub.set_display(ColorTransform::between(
            ColorSpace::SRGB,
            ColorSpace::DISPLAY_P3,
        ));
        assert_eq!(
            tile_count(&hub.frame("doc-1", &view(), &[], false, long).unwrap()),
            6
        );
        hub.close("doc-1");
        assert!(matches!(
            hub.frame("doc-1", &view(), &[], false, long),
            Err(HubError::NotOpen)
        ));
    }
}
