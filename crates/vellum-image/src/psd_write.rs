//! saving photoshop files (psd, and psb for big pictures).
//!
//! layers keep their names, groups (with pass through and whether they are open), masks,
//! clipping, blend modes, opacity, fill, visibility, colour labels and locks, so the file
//! opens in photoshop and photopea with the layers intact. 16 and 32 bit documents keep
//! their depth, with layers in the `Lr16` and `Lr32` blocks photoshop uses for them.
//!
//! photoshop's own formats for type, shapes, smart objects, layer styles and most
//! adjustments are large undocumented descriptors, so those layers are written as pixels
//! that look the same: text, shapes, fills and smart objects as what they draw, styles
//! drawn into their layer, and adjustments other than invert, posterize, threshold and
//! levels as what they make of the layers below them. vector masks are merged into the
//! layer mask. `ARCHITECTURE.md` section 26 lists what each kind becomes.

use std::io::Write;
use std::path::Path;

use vellum_gpu::{BlendMode, IRect};

use crate::adjustments;
use crate::document::{Content, ImageDocument, Label, Layer};
use crate::flatten;
use crate::raster::{Depth, Raster};

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn bei32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn len(out: &mut Vec<u8>, big: bool, v: u64) {
    if big {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        be32(out, v as u32);
    }
}

/// photoshop's run length encoding of one row
pub fn pack_bits(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 3 {
            out.push((1 - run as i32) as i8 as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        let start = i;
        while i < row.len() && i - start < 128 {
            if i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2] {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&row[start..i]);
    }
    out
}

fn encode_value(out: &mut Vec<u8>, v: f32, bits: u32) {
    match bits {
        8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
        16 => be16(out, (v.clamp(0.0, 1.0) * 65535.0).round() as u16),
        _ => out.extend_from_slice(&v.to_be_bytes()),
    }
}

/// one channel: the compression tag and its data. 8 and 16 bit channels are run length
/// encoded; 32 bit ones are stored raw, which every reader takes.
fn channel(values: &[f32], w: usize, h: usize, bits: u32, big: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity(values.len() * (bits as usize / 8));
    for v in values {
        encode_value(&mut raw, *v, bits);
    }
    let mut out = Vec::new();
    if bits == 32 || w == 0 || h == 0 {
        be16(&mut out, 0);
        out.extend_from_slice(&raw);
        return out;
    }
    be16(&mut out, 1);
    let stride = w * (bits as usize / 8);
    let rows: Vec<Vec<u8>> = raw.chunks(stride).take(h).map(pack_bits).collect();
    for r in &rows {
        if big {
            be32(&mut out, r.len() as u32);
        } else {
            be16(&mut out, r.len() as u16);
        }
    }
    for r in rows {
        out.extend_from_slice(&r);
    }
    out
}

fn pascal(out: &mut Vec<u8>, s: &str, pad: usize) {
    // the old name field is mac roman; non ascii goes in the unicode block
    let bytes: Vec<u8> = s
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'_' })
        .take(255)
        .collect();
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    let used = bytes.len() + 1;
    out.resize(out.len() + used.div_ceil(pad) * pad - used, 0);
}

fn block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(key);
    let padded = data.len().div_ceil(4) * 4;
    be32(out, padded as u32);
    out.extend_from_slice(data);
    out.resize(out.len() + padded - data.len(), 0);
}

pub fn blend_key(mode: BlendMode) -> [u8; 4] {
    *match mode {
        BlendMode::Normal => b"norm",
        BlendMode::Dissolve => b"diss",
        BlendMode::Darken => b"dark",
        BlendMode::Multiply => b"mul ",
        BlendMode::ColorBurn => b"idiv",
        BlendMode::LinearBurn => b"lbrn",
        BlendMode::DarkerColor => b"dkCl",
        BlendMode::Lighten => b"lite",
        BlendMode::Screen => b"scrn",
        BlendMode::ColorDodge => b"div ",
        BlendMode::LinearDodge => b"lddg",
        BlendMode::LighterColor => b"lgCl",
        BlendMode::Overlay => b"over",
        BlendMode::SoftLight => b"sLit",
        BlendMode::HardLight => b"hLit",
        BlendMode::VividLight => b"vLit",
        BlendMode::LinearLight => b"lLit",
        BlendMode::PinLight => b"pLit",
        BlendMode::HardMix => b"hMix",
        BlendMode::Difference => b"diff",
        BlendMode::Exclusion => b"smud",
        BlendMode::Subtract => b"fsub",
        BlendMode::Divide => b"fdiv",
        BlendMode::Hue => b"hue ",
        BlendMode::Saturation => b"sat ",
        BlendMode::Color => b"colr",
        BlendMode::Luminosity => b"lum ",
    }
}

fn label_code(l: Label) -> u16 {
    match l {
        Label::None => 0,
        Label::Red => 1,
        Label::Orange => 2,
        Label::Yellow => 3,
        Label::Green => 4,
        Label::Blue => 5,
        Label::Violet => 6,
        Label::Gray => 7,
    }
}

/// one layer record as it goes into the file
struct Rec {
    name: String,
    rect: IRect,
    /// straight rgba over `rect`, row by row; empty for groups and adjustments
    pixels: Vec<[f32; 4]>,
    blend: [u8; 4],
    opacity: u8,
    fill: u8,
    hidden: bool,
    clipped: bool,
    /// grey values over its own rect, the value outside, and whether it is off
    mask: Option<(IRect, Vec<f32>, u8, bool)>,
    label: u16,
    locks: u32,
    section: Option<u32>,
    /// an adjustment written as photoshop's own: its key and data
    adjustment: Option<([u8; 4], Vec<u8>)>,
}

impl Rec {
    fn empty(name: &str) -> Self {
        Self {
            name: name.into(),
            rect: IRect::EMPTY,
            pixels: Vec::new(),
            blend: *b"norm",
            opacity: 255,
            fill: 255,
            hidden: false,
            clipped: false,
            mask: None,
            label: 0,
            locks: 0,
            section: None,
            adjustment: None,
        }
    }
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// the photoshop data for the adjustments it can read back as adjustments
fn native_adjustment(adj: &adjustments::Adjustment) -> Option<([u8; 4], Vec<u8>)> {
    let reg = crate::compose::registry();
    let info = reg.get(&adj.kind)?.info();
    let p = vellum_gpu::params::Params {
        specs: info.params,
        values: &adj.params,
    };
    match adj.kind.as_str() {
        "effect.invert" => Some((*b"nvrt", Vec::new())),
        "effect.posterize" => {
            let mut d = Vec::new();
            be16(&mut d, p.int("levels").clamp(2, 255) as u16);
            be16(&mut d, 0);
            Some((*b"post", d))
        }
        "effect.threshold" => {
            let mut d = Vec::new();
            be16(&mut d, p.float("level").round().clamp(1.0, 255.0) as u16);
            be16(&mut d, 0);
            Some((*b"thrs", d))
        }
        "effect.levels" => {
            let mut d = Vec::new();
            be16(&mut d, 2);
            let set = |d: &mut Vec<u8>, ib: f32, iw: f32, ob: f32, ow: f32, g: f32| {
                be16(d, ib.round().clamp(0.0, 253.0) as u16);
                be16(d, iw.round().clamp(2.0, 255.0) as u16);
                be16(d, ob.round().clamp(0.0, 255.0) as u16);
                be16(d, ow.round().clamp(0.0, 255.0) as u16);
                be16(d, (g * 100.0).round().clamp(10.0, 999.0) as u16);
            };
            let values = (
                p.float("input_black"),
                p.float("input_white"),
                p.float("output_black"),
                p.float("output_white"),
                p.float("gamma"),
            );
            let channel = p.choice_index("channel") as usize;
            for i in 0..29 {
                if i == channel {
                    set(&mut d, values.0, values.1, values.2, values.3, values.4);
                } else {
                    set(&mut d, 0.0, 255.0, 0.0, 255.0, 1.0);
                }
            }
            Some((*b"levl", d))
        }
        _ => None,
    }
}

struct Writer<'a> {
    doc: &'a ImageDocument,
}

impl Writer<'_> {
    /// what a layer draws on its own (content, style, but not its masks or blend), straight
    fn solo(&self, layer: &Layer, styled: bool) -> Result<(IRect, Vec<[f32; 4]>), String> {
        let raster = match &layer.content {
            Content::Pixels(r) if !styled => r.clone(),
            _ => {
                let mut solo = self.doc.clone();
                solo.quick_mask = false;
                solo.selection = None;
                solo.layers = vec![Layer {
                    visible: true,
                    opacity: 1.0,
                    fill: 1.0,
                    blend: BlendMode::Normal,
                    clipped: false,
                    mask: None,
                    vector_mask: None,
                    ..layer.clone()
                }];
                // fills cover the canvas; a style can reach past the layer's pixels
                let area = match &layer.content {
                    Content::Fill(_) => self.doc.canvas(),
                    _ if styled => layer.bounds().expand(64),
                    _ => layer.bounds(),
                };
                flatten::merged(&solo, area, self.doc.depth)?
            }
        };
        Ok(read_rgba(&raster))
    }

    /// the layer mask and vector mask together, as photoshop's single user mask
    fn mask(&self, layer: &Layer) -> Option<(IRect, Vec<f32>, u8, bool)> {
        let lm = layer.mask.as_ref();
        let vm = layer.vector_mask.as_ref().filter(|v| v.mask.enabled);
        if lm.is_none() && vm.is_none() {
            return None;
        }
        let canvas = self.doc.canvas();
        let fill_l = lm.map_or(1.0, |m| m.raster.fill());
        let fill_v = vm.map_or(1.0, |v| v.raster.fill());
        let outside = fill_l * fill_v;
        let mut rect = IRect::EMPTY;
        if let Some(m) = lm {
            rect = rect.union(&m.raster.content_bounds());
        }
        if let Some(v) = vm {
            rect = rect.union(&v.raster.content_bounds());
        }
        // a mask that is the same everywhere still needs a pixel to be valid
        let rect = if rect.is_empty() {
            IRect::new(canvas.x, canvas.y, 1, 1)
        } else {
            rect
        };
        let mut values = Vec::with_capacity(rect.area() as usize);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let a = lm.map_or(1.0, |m| m.raster.get(x, y, 0));
                let b = vm.map_or(1.0, |v| v.raster.get(x, y, 0));
                values.push(a * b);
            }
        }
        let disabled = lm.is_some_and(|m| !m.enabled) && vm.is_none();
        Some((rect, values, to8(outside), disabled))
    }

    fn common(&self, layer: &Layer, rec: &mut Rec) {
        rec.blend = blend_key(layer.blend);
        rec.opacity = to8(layer.opacity);
        rec.fill = to8(layer.fill);
        rec.hidden = !layer.visible;
        rec.clipped = layer.clipped;
        rec.label = label_code(layer.label);
        rec.locks = u32::from(layer.locks.transparency)
            | u32::from(layer.locks.pixels) << 1
            | u32::from(layer.locks.position) << 2
            | u32::from(layer.locks.all) << 31;
        rec.mask = self.mask(layer);
    }

    fn layers(&self, list: &[Layer], out: &mut Vec<Rec>) -> Result<(), String> {
        for layer in list {
            match &layer.content {
                Content::Group {
                    children,
                    expanded,
                    pass_through,
                } => {
                    let mut end = Rec::empty("</Layer group>");
                    end.section = Some(3);
                    end.hidden = !layer.visible;
                    out.push(end);
                    self.layers(children, out)?;
                    let mut rec = Rec::empty(&layer.name);
                    self.common(layer, &mut rec);
                    if *pass_through && layer.blend == BlendMode::Normal {
                        rec.blend = *b"pass";
                    }
                    rec.section = Some(if *expanded { 1 } else { 2 });
                    out.push(rec);
                }
                Content::Adjustment(adj) => {
                    let mut rec = Rec::empty(&layer.name);
                    self.common(layer, &mut rec);
                    if let Some(native) = native_adjustment(adj) {
                        rec.adjustment = Some(native);
                    } else {
                        // what the adjustment makes of everything below it, at full strength;
                        // its mask, opacity and blend mode still apply in photoshop
                        let mut below = adjustments::below(self.doc, layer.id);
                        below.layers.push(Layer {
                            visible: true,
                            opacity: 1.0,
                            fill: 1.0,
                            blend: BlendMode::Normal,
                            clipped: false,
                            mask: None,
                            vector_mask: None,
                            ..layer.clone()
                        });
                        let r = flatten::merged(&below, self.doc.canvas(), self.doc.depth)?;
                        (rec.rect, rec.pixels) = read_rgba(&r);
                        rec.name = format!("{} (applied)", layer.name);
                    }
                    out.push(rec);
                }
                _ => {
                    let styled = layer.style.as_ref().is_some_and(|s| s.shows());
                    let mut rec = Rec::empty(&layer.name);
                    self.common(layer, &mut rec);
                    (rec.rect, rec.pixels) = self.solo(layer, styled)?;
                    out.push(rec);
                }
            }
        }
        Ok(())
    }
}

fn read_rgba(r: &Raster) -> (IRect, Vec<[f32; 4]>) {
    let rect = r.content_bounds();
    if rect.is_empty() {
        return (IRect::EMPTY, Vec::new());
    }
    let v = r.read(rect);
    (
        rect,
        v.chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
    )
}

fn layer_info(recs: &[Rec], bits: u32, big: bool) -> Vec<u8> {
    let mut records = Vec::new();
    let mut data = Vec::new();
    for l in recs {
        let (w, h) = (l.rect.w as usize, l.rect.h as usize);
        let pick = |k: usize| l.pixels.iter().map(|p| p[k]).collect::<Vec<f32>>();
        let mut chans: Vec<(i16, Vec<u8>)> = vec![(-1, channel(&pick(3), w, h, bits, big))];
        for k in 0..3 {
            chans.push((k as i16, channel(&pick(k), w, h, bits, big)));
        }
        if let Some((mr, values, ..)) = &l.mask {
            chans.push((-2, channel(values, mr.w as usize, mr.h as usize, bits, big)));
        }
        let r = l.rect;
        bei32(&mut records, r.y);
        bei32(&mut records, r.x);
        bei32(&mut records, r.bottom());
        bei32(&mut records, r.right());
        be16(&mut records, chans.len() as u16);
        for (id, d) in &chans {
            records.extend_from_slice(&id.to_be_bytes());
            len(&mut records, big, d.len() as u64);
        }
        records.extend_from_slice(b"8BIM");
        records.extend_from_slice(&l.blend);
        records.push(l.opacity);
        records.push(u8::from(l.clipped));
        // bit 1 hides the layer
        records.push(if l.hidden { 2 } else { 0 });
        records.push(0);
        let mut extra = Vec::new();
        match &l.mask {
            Some((mr, _, default, disabled)) => {
                be32(&mut extra, 20);
                bei32(&mut extra, mr.y);
                bei32(&mut extra, mr.x);
                bei32(&mut extra, mr.bottom());
                bei32(&mut extra, mr.right());
                extra.push(*default);
                extra.push(if *disabled { 2 } else { 0 });
                extra.extend_from_slice(&[0, 0]);
            }
            None => be32(&mut extra, 0),
        }
        be32(&mut extra, 0);
        pascal(&mut extra, &l.name, 4);
        let mut luni = Vec::new();
        let units: Vec<u16> = l.name.encode_utf16().collect();
        be32(&mut luni, units.len() as u32);
        for u in units {
            be16(&mut luni, u);
        }
        block(&mut extra, b"luni", &luni);
        if let Some(s) = l.section {
            let mut d = Vec::new();
            be32(&mut d, s);
            if s != 3 {
                d.extend_from_slice(b"8BIM");
                d.extend_from_slice(&l.blend);
            }
            block(&mut extra, b"lsct", &d);
        }
        if l.fill != 255 {
            block(&mut extra, b"iOpa", &[l.fill, 0, 0, 0]);
        }
        if l.label != 0 {
            let mut d = Vec::new();
            be16(&mut d, l.label);
            d.extend_from_slice(&[0; 6]);
            block(&mut extra, b"lclr", &d);
        }
        if l.locks != 0 {
            let mut d = Vec::new();
            be32(&mut d, l.locks);
            block(&mut extra, b"lspf", &d);
        }
        if let Some((key, d)) = &l.adjustment {
            block(&mut extra, key, d);
        }
        be32(&mut records, extra.len() as u32);
        records.extend_from_slice(&extra);
        for (_, d) in chans {
            data.extend_from_slice(&d);
        }
    }
    let mut info = Vec::new();
    // a negative count says the first alpha channel is the merged picture's transparency
    be16(&mut info, (-(recs.len() as i32)) as i16 as u16);
    info.extend_from_slice(&records);
    info.extend_from_slice(&data);
    if info.len() % 2 == 1 {
        info.push(0);
    }
    info
}

/// the whole file. `big` writes a psb, which photoshop needs past 30000 pixels a side.
pub fn encode(doc: &ImageDocument, big: bool) -> Result<Vec<u8>, String> {
    if !big && (doc.width > 30_000 || doc.height > 30_000) {
        return Err("Pictures over 30,000 pixels a side need the PSB format.".into());
    }
    let bits = doc.depth.bits();
    let w = Writer { doc };
    let mut recs = Vec::new();
    w.layers(&doc.layers, &mut recs)?;

    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    be16(&mut out, if big { 2 } else { 1 });
    out.extend_from_slice(&[0; 6]);
    be16(&mut out, 4);
    be32(&mut out, doc.height);
    be32(&mut out, doc.width);
    be16(&mut out, bits as u16);
    be16(&mut out, 3);
    be32(&mut out, 0);

    let mut res = Vec::new();
    let mut resource = |id: u16, data: &[u8]| {
        res.extend_from_slice(b"8BIM");
        be16(&mut res, id);
        res.extend_from_slice(&[0, 0]);
        be32(&mut res, data.len() as u32);
        res.extend_from_slice(data);
        if data.len() % 2 == 1 {
            res.push(0);
        }
    };
    let mut d = Vec::new();
    let r = (doc.resolution * 65536.0) as u32;
    be32(&mut d, r);
    be16(&mut d, 1);
    be16(&mut d, 1);
    be32(&mut d, r);
    be16(&mut d, 1);
    be16(&mut d, 1);
    resource(1005, &d);
    if !doc.guides.is_empty() {
        let mut d = Vec::new();
        be32(&mut d, 1);
        be32(&mut d, 576);
        be32(&mut d, 576);
        be32(&mut d, doc.guides.len() as u32);
        for g in &doc.guides {
            bei32(&mut d, (g.position * 32.0).round() as i32);
            d.push(u8::from(!g.vertical));
        }
        resource(1032, &d);
    }
    if let Some(icc) = &doc.color.icc {
        resource(1039, icc);
    }
    be32(&mut out, res.len() as u32);
    out.extend_from_slice(&res);

    let mut section = Vec::new();
    if recs.is_empty() {
        len(&mut section, big, 0);
        be32(&mut section, 0);
    } else if bits == 8 {
        let info = layer_info(&recs, bits, big);
        len(&mut section, big, info.len() as u64);
        section.extend_from_slice(&info);
        be32(&mut section, 0);
    } else {
        // photoshop keeps deep layers in a tagged block after an empty layer info
        len(&mut section, big, 0);
        be32(&mut section, 0);
        let key = if bits == 32 { b"Lr32" } else { b"Lr16" };
        let info = layer_info(&recs, bits, big);
        section.extend_from_slice(b"8BIM");
        section.extend_from_slice(key);
        let padded = info.len().div_ceil(4) * 4;
        len(&mut section, big, padded as u64);
        section.extend_from_slice(&info);
        section.resize(section.len() + padded - info.len(), 0);
    }
    len(&mut out, big, section.len() as u64);
    out.extend_from_slice(&section);

    // the flattened picture, for previews and apps that do not read layers. photoshop
    // stores it matted on white where it is see through.
    let canvas = doc.canvas();
    let merged = flatten::merged(doc, canvas, doc.depth)?;
    let px = merged.read(canvas);
    let linear = doc.depth == Depth::F32;
    let mut planes: [Vec<f32>; 4] = Default::default();
    for p in px.chunks_exact(4) {
        let a = p[3];
        for k in 0..3 {
            let v = if linear { p[k] } else { p[k] * a + (1.0 - a) };
            planes[k].push(v);
        }
        planes[3].push(a);
    }
    let (cw, ch) = (canvas.w as usize, canvas.h as usize);
    if bits == 32 {
        be16(&mut out, 0);
        for plane in &planes {
            for v in plane {
                encode_value(&mut out, *v, bits);
            }
        }
    } else {
        be16(&mut out, 1);
        let mut rows = Vec::new();
        for plane in &planes {
            let mut raw = Vec::new();
            for v in plane {
                encode_value(&mut raw, *v, bits);
            }
            let stride = cw * (bits as usize / 8);
            for row in raw.chunks(stride.max(1)).take(ch) {
                rows.push(pack_bits(row));
            }
        }
        for r in &rows {
            if big {
                be32(&mut out, r.len() as u32);
            } else {
                be16(&mut out, r.len() as u16);
            }
        }
        for r in rows {
            out.extend_from_slice(&r);
        }
    }
    Ok(out)
}

/// saves a photoshop file, through a temporary name so a failed save leaves the old file
pub fn save(doc: &ImageDocument, path: &Path) -> Result<(), String> {
    let big = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("psb"));
    let bytes = encode(doc, big)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|err| format!("The file could not be written: {err}"))?;
    tmp.write_all(&bytes)
        .map_err(|err| format!("The file could not be written: {err}"))?;
    tmp.persist(path)
        .map_err(|err| format!("The file could not be saved: {}", err.error))?;
    Ok(())
}
