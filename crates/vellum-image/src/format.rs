//! storing image documents: the json manifest, binary raster files, and the native
//! `.vellum` file.
//!
//! a document is a manifest ([`ImageState`], also what the window reads to draw the
//! layers panel) plus one binary file per raster. autosave writes those files into the
//! recovery folder and only rewrites the ones that changed. a `.vellum` file is the same
//! thing in a zip archive:
//!
//! ```text
//! manifest.json        the document, every layer and its settings
//! pixels/layer-3.bin   each layer's pixels, layer masks, the selection, saved selections
//! profile.icc          the colour profile, when the picture had one
//! thumbnail.png        a small preview for file browsers and the home screen
//! ```
//!
//! entries are stored, not compressed again, because each raster file is already
//! compressed tile by tile. unzipping a `.vellum` file gives a folder vellum also opens.

use std::hash::Hasher;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_core::documents::{DocumentError, DocumentModel, StateFile};
use vellum_gpu::{BlendMode, IRect};

use crate::document::{
    Content, DocColor, EditTarget, Guide, ImageDocument, Label, Layer, LayerId, LayerMask, Locks,
    SavedChannel,
};
use crate::raster::{Channels, Depth, Raster, TILE, TileData};

/// the manifest's version; older ones still open
pub const FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 4] = b"VRST";

/// a raster file named in the manifest
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RasterRef {
    pub file: String,
    pub fill: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaskState {
    pub raster: RasterRef,
    pub enabled: bool,
    pub linked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LayerState {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub fill: f32,
    pub blend: BlendMode,
    #[serde(default)]
    pub pass_through: bool,
    #[serde(default)]
    pub locks: Locks,
    #[serde(default)]
    pub label: Label,
    #[serde(default)]
    pub clipped: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub group: bool,
    #[serde(default)]
    pub expanded: bool,
    /// bottom to top, like the document's own list
    #[serde(default)]
    pub children: Vec<LayerState>,
    #[serde(default)]
    pub pixels: Option<RasterRef>,
    #[serde(default)]
    pub mask: Option<MaskState>,
    /// where the layer has pixels, for the window's move handles and snapping
    #[serde(default)]
    pub bounds: IRect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChannelState {
    pub id: u32,
    pub name: String,
    pub raster: RasterRef,
}

/// the whole document without its pixels
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageState {
    /// always "image", so other tools can tell what a `.vellum` file holds
    pub kind: String,
    pub format: u32,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub depth: Depth,
    pub resolution: f32,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub profile_file: Option<String>,
    pub layers: Vec<LayerState>,
    #[serde(default)]
    pub active: Option<LayerId>,
    #[serde(default)]
    pub target: EditTarget,
    #[serde(default)]
    pub selection: Option<RasterRef>,
    /// where the selection is, for the window
    #[serde(default)]
    pub selection_bounds: Option<IRect>,
    #[serde(default)]
    pub last_selection: Option<RasterRef>,
    #[serde(default)]
    pub channels: Vec<ChannelState>,
    #[serde(default)]
    pub quick_mask: bool,
    #[serde(default)]
    pub guides: Vec<Guide>,
    pub next_id: u32,
    /// the file still loading in the background, if any
    #[serde(default)]
    pub loading: Option<String>,
}

/// the manifest and every raster it names
pub fn describe(doc: &ImageDocument) -> (ImageState, Vec<(String, Raster)>) {
    let mut files = Vec::new();
    let mut add = |name: String, raster: &Raster| -> RasterRef {
        files.push((name.clone(), raster.clone()));
        RasterRef {
            file: name,
            fill: raster.fill(),
        }
    };
    fn layer(l: &Layer, add: &mut dyn FnMut(String, &Raster) -> RasterRef) -> LayerState {
        let (group, expanded, pass_through, children, pixels) = match &l.content {
            Content::Pixels(r) => (
                false,
                false,
                false,
                Vec::new(),
                Some(add(format!("layer-{}.bin", l.id.0), r)),
            ),
            Content::Group {
                children,
                expanded,
                pass_through,
            } => (
                true,
                *expanded,
                *pass_through,
                children.iter().map(|c| layer(c, add)).collect(),
                None,
            ),
        };
        LayerState {
            id: l.id,
            name: l.name.clone(),
            visible: l.visible,
            opacity: l.opacity,
            fill: l.fill,
            blend: l.blend,
            pass_through,
            locks: l.locks,
            label: l.label,
            clipped: l.clipped,
            origin: l.origin.clone(),
            group,
            expanded,
            children,
            pixels,
            mask: l.mask.as_ref().map(|m| MaskState {
                raster: add(format!("mask-{}.bin", l.id.0), &m.raster),
                enabled: m.enabled,
                linked: m.linked,
            }),
            bounds: l.bounds(),
        }
    }
    let layers = doc.layers.iter().map(|l| layer(l, &mut add)).collect();
    let selection = doc
        .selection
        .as_ref()
        .map(|s| add("selection.bin".into(), s));
    let last_selection = doc
        .last_selection
        .as_ref()
        .map(|s| add("last-selection.bin".into(), s));
    let channels = doc
        .channels
        .iter()
        .map(|c| ChannelState {
            id: c.id,
            name: c.name.clone(),
            raster: add(format!("channel-{}.bin", c.id), &c.raster),
        })
        .collect();
    let state = ImageState {
        kind: "image".into(),
        format: FORMAT_VERSION,
        title: doc.title.clone(),
        width: doc.width,
        height: doc.height,
        depth: doc.depth,
        resolution: doc.resolution,
        profile: doc.color.name.clone(),
        profile_file: doc.color.icc.as_ref().map(|_| "profile.icc".to_owned()),
        layers,
        active: doc.active,
        target: doc.target,
        selection,
        selection_bounds: doc.selection.as_ref().map(Raster::content_bounds),
        last_selection,
        channels,
        quick_mask: doc.quick_mask,
        guides: doc.guides.clone(),
        next_id: doc.next_id,
        loading: doc.loading.clone(),
    };
    (state, files)
}

/// rebuilds a document from its manifest, reading named files through `read`
pub fn assemble(
    state: ImageState,
    read: &dyn Fn(&str) -> std::io::Result<Vec<u8>>,
) -> Result<ImageDocument, String> {
    if state.kind != "image" {
        return Err(format!(
            "This project holds a {} document, which this version of Vellum cannot open.",
            state.kind
        ));
    }
    let load = |r: &RasterRef, channels: Channels| -> Result<Raster, String> {
        let bytes = read(&r.file)
            .map_err(|e| format!("Part of the document ({}) could not be read: {e}", r.file))?;
        let raster = read_raster(&bytes)?;
        if raster.channels() != channels {
            return Err(format!("Part of the document ({}) is damaged.", r.file));
        }
        Ok(raster)
    };
    fn layer(
        s: LayerState,
        load: &dyn Fn(&RasterRef, Channels) -> Result<Raster, String>,
        depth: Depth,
    ) -> Result<Layer, String> {
        let content = if s.group {
            Content::Group {
                children: s
                    .children
                    .into_iter()
                    .map(|c| layer(c, load, depth))
                    .collect::<Result<_, _>>()?,
                expanded: s.expanded,
                pass_through: s.pass_through,
            }
        } else {
            Content::Pixels(match &s.pixels {
                Some(r) => load(r, Channels::Rgba)?,
                None => Raster::rgba(depth),
            })
        };
        let mask = match s.mask {
            Some(m) => Some(LayerMask {
                raster: load(&m.raster, Channels::Gray)?,
                enabled: m.enabled,
                linked: m.linked,
            }),
            None => None,
        };
        Ok(Layer {
            id: s.id,
            name: s.name,
            visible: s.visible,
            opacity: s.opacity.clamp(0.0, 1.0),
            fill: s.fill.clamp(0.0, 1.0),
            blend: s.blend,
            locks: s.locks,
            label: s.label,
            clipped: s.clipped,
            mask,
            content,
            origin: s.origin,
        })
    }
    let mut doc = ImageDocument::empty(state.title, state.width, state.height, state.depth);
    doc.resolution = state.resolution;
    if let Some(file) = &state.profile_file
        && let Ok(bytes) = read(file)
    {
        doc.color = DocColor::from_icc(bytes);
    }
    doc.layers = state
        .layers
        .into_iter()
        .map(|l| layer(l, &load, state.depth))
        .collect::<Result<_, _>>()?;
    doc.active = state.active.filter(|id| doc.layer(*id).is_some());
    doc.target = state.target;
    doc.selection = state
        .selection
        .as_ref()
        .map(|r| load(r, Channels::Gray))
        .transpose()?;
    doc.last_selection = state
        .last_selection
        .as_ref()
        .map(|r| load(r, Channels::Gray))
        .transpose()?;
    doc.channels = state
        .channels
        .into_iter()
        .map(|c| {
            Ok(SavedChannel {
                id: c.id,
                name: c.name,
                raster: load(&c.raster, Channels::Gray)?,
            })
        })
        .collect::<Result<_, String>>()?;
    doc.quick_mask = state.quick_mask;
    doc.guides = state.guides;
    let highest = doc.all_layers().iter().map(|l| l.id.0).max().unwrap_or(0);
    doc.next_id = state.next_id.max(highest + 1);
    doc.loading = state.loading;
    Ok(doc)
}

/// a raster's file stamp: changes exactly when its pixels do
fn stamp(r: &Raster) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    h.write_u64(r.id());
    h.write_u32(r.fill().to_bits());
    for key in r.tile_keys() {
        h.write_i32(key.0);
        h.write_i32(key.1);
        h.write_u64(r.tile(key.0, key.1).map_or(0, |t| t.rev));
    }
    h.finish()
}

/// tile bytes, each value minus the same channel of the pixel to its left, so flat areas
/// become runs of zeros that compress well (png's sub filter)
fn filtered(data: &TileData, n: usize) -> Vec<u8> {
    let row = TILE as usize * n;
    match data {
        TileData::U8(d) => {
            let mut out = d.clone();
            for (i, v) in out.iter_mut().enumerate() {
                if i % row >= n {
                    *v = d[i].wrapping_sub(d[i - n]);
                }
            }
            out
        }
        TileData::U16(d) => {
            let mut out = Vec::with_capacity(d.len() * 2);
            for (i, v) in d.iter().enumerate() {
                let p = if i % row >= n {
                    v.wrapping_sub(d[i - n])
                } else {
                    *v
                };
                out.extend_from_slice(&p.to_le_bytes());
            }
            out
        }
        TileData::F32(d) => {
            let mut out = Vec::with_capacity(d.len() * 4);
            for (i, v) in d.iter().enumerate() {
                let bits = v.to_bits();
                let p = if i % row >= n {
                    bits.wrapping_sub(d[i - n].to_bits())
                } else {
                    bits
                };
                out.extend_from_slice(&p.to_le_bytes());
            }
            out
        }
    }
}

fn unfiltered(bytes: &[u8], depth: Depth, n: usize) -> Result<TileData, String> {
    let row = TILE as usize * n;
    let len = (TILE * TILE) as usize * n;
    let bad = || "A picture's pixels are damaged.".to_owned();
    Ok(match depth {
        Depth::U8 => {
            if bytes.len() != len {
                return Err(bad());
            }
            let mut d = bytes.to_vec();
            for i in 0..len {
                if i % row >= n {
                    d[i] = d[i].wrapping_add(d[i - n]);
                }
            }
            TileData::U8(d)
        }
        Depth::U16 => {
            if bytes.len() != len * 2 {
                return Err(bad());
            }
            let mut d: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            for i in 0..len {
                if i % row >= n {
                    d[i] = d[i].wrapping_add(d[i - n]);
                }
            }
            TileData::U16(d)
        }
        Depth::F32 => {
            if bytes.len() != len * 4 {
                return Err(bad());
            }
            let mut bits: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            for i in 0..len {
                if i % row >= n {
                    bits[i] = bits[i].wrapping_add(bits[i - n]);
                }
            }
            TileData::F32(bits.into_iter().map(f32::from_bits).collect())
        }
    })
}

/// a raster as bytes, little endian:
///
/// ```text
/// "VRST"  u32 version  u8 channels  u8 bytes per value  u16 0  f32 fill  u32 tiles
/// per tile: i32 x  i32 y  u32 length  deflated filtered tile bytes
/// ```
pub fn write_raster(r: &Raster, out: &mut dyn Write) -> std::io::Result<()> {
    let n = r.channels().n();
    let keys = r.tile_keys();
    let packed: Vec<std::io::Result<Vec<u8>>> = keys
        .par_iter()
        .map(|&(tx, ty)| {
            let tile = r.tile(tx, ty).map(|t| t.data.clone());
            let raw = tile.map(|d| filtered(&d, n)).unwrap_or_default();
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
            enc.write_all(&raw)?;
            enc.finish()
        })
        .collect();
    out.write_all(MAGIC)?;
    out.write_all(&1u32.to_le_bytes())?;
    let bytes_per = match r.depth() {
        Depth::U8 => 1u8,
        Depth::U16 => 2,
        Depth::F32 => 4,
    };
    out.write_all(&[n as u8, bytes_per, 0, 0])?;
    out.write_all(&r.fill().to_le_bytes())?;
    out.write_all(&(keys.len() as u32).to_le_bytes())?;
    for ((tx, ty), data) in keys.iter().zip(packed) {
        let data = data?;
        out.write_all(&tx.to_le_bytes())?;
        out.write_all(&ty.to_le_bytes())?;
        out.write_all(&(data.len() as u32).to_le_bytes())?;
        out.write_all(&data)?;
    }
    Ok(())
}

pub fn read_raster(bytes: &[u8]) -> Result<Raster, String> {
    let bad = || "A picture's pixels are damaged or from a newer Vellum.".to_owned();
    let take = |at: &mut usize, len: usize| -> Result<&[u8], String> {
        let s = bytes.get(*at..*at + len).ok_or_else(bad)?;
        *at += len;
        Ok(s)
    };
    let u32_at = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let mut at = 0;
    if take(&mut at, 4)? != MAGIC || u32_at(take(&mut at, 4)?) != 1 {
        return Err(bad());
    }
    let head = take(&mut at, 4)?;
    let channels = match head[0] {
        4 => Channels::Rgba,
        1 => Channels::Gray,
        _ => return Err(bad()),
    };
    let depth = match head[1] {
        1 => Depth::U8,
        2 => Depth::U16,
        4 => Depth::F32,
        _ => return Err(bad()),
    };
    let fill = f32::from_bits(u32_at(take(&mut at, 4)?));
    let count = u32_at(take(&mut at, 4)?) as usize;
    let mut entries = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let tx = u32_at(take(&mut at, 4)?) as i32;
        let ty = u32_at(take(&mut at, 4)?) as i32;
        let len = u32_at(take(&mut at, 4)?) as usize;
        entries.push((tx, ty, take(&mut at, len)?));
    }
    let n = channels.n();
    let tiles: Vec<Result<(i32, i32, TileData), String>> = entries
        .par_iter()
        .map(|(tx, ty, data)| {
            let mut raw = Vec::new();
            DeflateDecoder::new(*data)
                .read_to_end(&mut raw)
                .map_err(|_| bad())?;
            Ok((*tx, *ty, unfiltered(&raw, depth, n)?))
        })
        .collect();
    let mut r = Raster::new(channels, depth, fill);
    for t in tiles {
        let (tx, ty, data) = t?;
        r.set_tile(tx, ty, Some(Arc::new(data)));
    }
    Ok(r)
}

impl DocumentModel for ImageDocument {
    const TYPE: &'static str = "image";

    fn commands() -> vellum_core::command::CommandRegistry<Self> {
        crate::commands::registry()
    }

    fn to_state(&self) -> Result<serde_json::Value, DocumentError> {
        Ok(serde_json::to_value(describe(self).0)?)
    }

    fn from_state(state: serde_json::Value) -> Result<Self, DocumentError> {
        Self::from_saved_state(state, None)
    }

    fn state_files(&self) -> Vec<StateFile> {
        let (_, rasters) = describe(self);
        let mut files: Vec<StateFile> = rasters
            .into_iter()
            .map(|(name, r)| {
                let s = stamp(&r);
                StateFile::new(name, s, move |out| write_raster(&r, out))
            })
            .collect();
        if let Some(icc) = self.color.icc.clone() {
            let mut h = std::hash::DefaultHasher::new();
            h.write(&icc);
            files.push(StateFile::new("profile.icc", h.finish(), move |out| {
                out.write_all(&icc)
            }));
        }
        files
    }

    fn from_saved_state(
        state: serde_json::Value,
        files: Option<&Path>,
    ) -> Result<Self, DocumentError> {
        let state: ImageState = serde_json::from_value(state)?;
        let read = |name: &str| -> std::io::Result<Vec<u8>> {
            match files {
                Some(dir) => std::fs::read(dir.join(name)),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no files",
                )),
            }
        };
        assemble(state, &read).map_err(DocumentError::State)
    }

    /// documents opened from a file show the file's name, which follows "save as";
    /// new documents show their own name until they are saved
    fn title(&self) -> Option<String> {
        (!self.title.is_empty()).then(|| self.title.clone())
    }
}

/// writes a `.vellum` file, to a temporary name first so a failed save never damages the
/// file that was there. `thumbnail` is a png.
pub fn save(doc: &ImageDocument, path: &Path, thumbnail: Option<Vec<u8>>) -> Result<(), String> {
    let (state, rasters) = describe(doc);
    let fail = |e: &dyn std::fmt::Display| format!("{} could not be saved: {e}", path.display());
    let partial = path.with_extension("vellum-part");
    let written = (|| -> Result<(), String> {
        let file = std::fs::File::create(&partial).map_err(|e| fail(&e))?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let stored = |big: bool| {
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(big)
        };
        let manifest = serde_json::to_vec_pretty(&state).map_err(|e| fail(&e))?;
        zip.start_file("manifest.json", stored(false))
            .map_err(|e| fail(&e))?;
        zip.write_all(&manifest).map_err(|e| fail(&e))?;
        for (name, raster) in &rasters {
            let mut bytes = Vec::new();
            write_raster(raster, &mut bytes).map_err(|e| fail(&e))?;
            zip.start_file(
                format!("pixels/{name}"),
                stored(bytes.len() as u64 >= u64::from(u32::MAX)),
            )
            .map_err(|e| fail(&e))?;
            zip.write_all(&bytes).map_err(|e| fail(&e))?;
        }
        if let Some(icc) = &doc.color.icc {
            zip.start_file("profile.icc", stored(false))
                .map_err(|e| fail(&e))?;
            zip.write_all(icc).map_err(|e| fail(&e))?;
        }
        if let Some(png) = thumbnail {
            zip.start_file("thumbnail.png", stored(false))
                .map_err(|e| fail(&e))?;
            zip.write_all(&png).map_err(|e| fail(&e))?;
        }
        let out = zip.finish().map_err(|e| fail(&e))?;
        let file = out.into_inner().map_err(|e| fail(&e.error()))?;
        file.sync_all().map_err(|e| fail(&e))?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    std::fs::rename(&partial, path).map_err(|e| fail(&e))
}

/// opens a `.vellum` file, or a folder unzipped from one
pub fn open(path: &Path) -> Result<ImageDocument, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{} could not be opened: {e}", path.display());
    if path.is_dir() {
        let manifest = std::fs::read(path.join("manifest.json")).map_err(|e| fail(&e))?;
        let state: ImageState = serde_json::from_slice(&manifest).map_err(|e| fail(&e))?;
        let read = |name: &str| {
            let p = if name == "profile.icc" {
                path.join(name)
            } else {
                path.join("pixels").join(name)
            };
            std::fs::read(p)
        };
        return assemble(state, &read);
    }
    let file = std::fs::File::open(path).map_err(|e| fail(&e))?;
    let archive = parking_lot::Mutex::new(
        zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| fail(&e))?,
    );
    let entry = |name: &str| -> std::io::Result<Vec<u8>> {
        let mut zip = archive.lock();
        let mut f = zip
            .by_name(name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;
        let mut out = Vec::new();
        f.read_to_end(&mut out)?;
        Ok(out)
    };
    let manifest = entry("manifest.json").map_err(|e| fail(&e))?;
    let state: ImageState = serde_json::from_slice(&manifest).map_err(|e| fail(&e))?;
    let read = |name: &str| {
        if name == "profile.icc" {
            entry(name)
        } else {
            entry(&format!("pixels/{name}"))
        }
    };
    assemble(state, &read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Combine, Shape, combine};

    fn sample() -> ImageDocument {
        let mut doc =
            ImageDocument::new("poster", 300, 200, Depth::U16, Some([0.2, 0.4, 0.6, 1.0]));
        let id = doc.new_id();
        let mut r = Raster::rgba(Depth::U16);
        r.update(IRect::new(-20, 10, 120, 50), |x, y, px| {
            px.copy_from_slice(&[x as f32 / 300.0, y as f32 / 200.0, 0.5, 0.75]);
        });
        let mut layer = Layer::pixels(id, "Paint", r);
        layer.blend = BlendMode::Multiply;
        layer.mask = Some(LayerMask {
            raster: Raster::gray(Depth::U16, 1.0),
            enabled: true,
            linked: false,
        });
        let gid = doc.new_id();
        doc.layers.push(Layer::group(gid, "Group", vec![layer]));
        let shape = Shape::Ellipse {
            x: 10.0,
            y: 10.0,
            w: 80.0,
            h: 50.0,
        };
        doc.selection = combine(
            None,
            doc.canvas(),
            shape.bounds(),
            Combine::Replace,
            &*shape.coverage(true),
        );
        doc
    }

    fn same_pixels(a: &Raster, b: &Raster) {
        assert_eq!(a.tile_keys(), b.tile_keys());
        for (tx, ty) in a.tile_keys() {
            assert_eq!(a.tile(tx, ty).unwrap().data, b.tile(tx, ty).unwrap().data);
        }
    }

    #[test]
    fn rasters_round_trip_exactly_at_every_depth() {
        for depth in [Depth::U8, Depth::U16, Depth::F32] {
            let mut r = Raster::rgba(depth);
            r.update(IRect::new(-300, 5, 700, 40), |x, y, px| {
                px.copy_from_slice(&[(x as f32).sin().abs(), y as f32 / 50.0, 0.25, 1.0]);
            });
            let mut bytes = Vec::new();
            write_raster(&r, &mut bytes).unwrap();
            same_pixels(&r, &read_raster(&bytes).unwrap());
        }
        assert!(read_raster(b"VRST junk").is_err());
    }

    #[test]
    fn vellum_files_keep_everything() {
        let doc = sample();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("poster.vellum");
        save(&doc, &path, None).unwrap();
        let back = open(&path).unwrap();
        assert_eq!(describe(&back).0, describe(&doc).0);
        let (a, b) = (&doc.layers[1].children()[0], &back.layers[1].children()[0]);
        same_pixels(a.raster().unwrap(), b.raster().unwrap());
        assert_eq!(b.mask.as_ref().unwrap().raster.fill(), 1.0);
        same_pixels(
            doc.selection.as_ref().unwrap(),
            back.selection.as_ref().unwrap(),
        );
        assert!(!dir.path().join("poster.vellum-part").exists());
    }

    #[test]
    fn state_files_restore_a_document() {
        let doc = sample();
        let dir = tempfile::tempdir().unwrap();
        for f in doc.state_files() {
            let mut out = std::fs::File::create(dir.path().join(&f.name)).unwrap();
            (f.write)(&mut out).unwrap();
        }
        let state = doc.to_state().unwrap();
        let back = ImageDocument::from_saved_state(state, Some(dir.path())).unwrap();
        assert_eq!(describe(&back).0, describe(&doc).0);
        // the same pixels give the same stamps, so autosave would not write them again
        let stamps = |d: &ImageDocument| {
            d.state_files()
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(stamps(&doc), stamps(&back));
    }
}
