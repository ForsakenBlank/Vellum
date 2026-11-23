//! a small photoshop file writer, only for testing the reader. it writes what the reader
//! is meant to keep (layers, groups, masks, clipping, blend modes, opacity, fill,
//! visibility, locks, labels, type layers, guides, resolution, the colour profile) at 8,
//! 16 or 32 bits, as psd or psb, with any of the four channel compressions.
//!
//! it follows adobe's specification rather than the reader's code, so the two are checked
//! against the format and not just against each other.

#![allow(dead_code)]

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// how channels are stored
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packing {
    Raw = 0,
    Rle = 1,
    Zip = 2,
    ZipPredicted = 3,
}

#[derive(Debug, Clone)]
pub struct WMask {
    /// x, y, w, h
    pub rect: (i32, i32, u32, u32),
    pub values: Vec<f32>,
    /// 0 or 255, what is outside the rect
    pub default: u8,
    pub disabled: bool,
}

/// one layer record, bottom to top. a group is an end marker (`section: Some(3)`) below
/// its children and the group's own record (`section: Some(1)` open, `Some(2)` closed)
/// above them, as photoshop writes them.
#[derive(Debug, Clone)]
pub struct WLayer {
    pub name: String,
    pub rect: (i32, i32, u32, u32),
    /// straight rgba, row by row; empty for groups
    pub pixels: Vec<[f32; 4]>,
    pub blend: [u8; 4],
    pub opacity: u8,
    pub fill: Option<u8>,
    pub hidden: bool,
    pub clipped: bool,
    pub mask: Option<WMask>,
    pub label: u16,
    pub locks: u32,
    pub text: bool,
    pub section: Option<u32>,
}

impl WLayer {
    pub fn pixels(
        name: &str,
        rect: (i32, i32, u32, u32),
        f: impl Fn(u32, u32) -> [f32; 4],
    ) -> Self {
        let mut pixels = Vec::new();
        for y in 0..rect.3 {
            for x in 0..rect.2 {
                pixels.push(f(x, y));
            }
        }
        Self {
            name: name.into(),
            rect,
            pixels,
            blend: *b"norm",
            opacity: 255,
            fill: None,
            hidden: false,
            clipped: false,
            mask: None,
            label: 0,
            locks: 0,
            text: false,
            section: None,
        }
    }

    pub fn group_end() -> Self {
        Self {
            section: Some(3),
            ..Self::pixels("</Layer group>", (0, 0, 0, 0), |_, _| [0.0; 4])
        }
    }

    pub fn group(name: &str, blend: [u8; 4], open: bool) -> Self {
        Self {
            section: Some(if open { 1 } else { 2 }),
            blend,
            ..Self::pixels(name, (0, 0, 0, 0), |_, _| [0.0; 4])
        }
    }
}

pub struct WDoc {
    pub width: u32,
    pub height: u32,
    pub bits: u16,
    pub big: bool,
    pub gray: bool,
    pub layers: Vec<WLayer>,
    /// (position in 1/32 pixel, 0 vertical or 1 horizontal)
    pub guides: Vec<(i32, u8)>,
    pub icc: Option<Vec<u8>>,
    pub resolution: Option<f32>,
    pub packing: Packing,
    /// 16 and 32 bit layers go in a tagged block after the (empty) layer info, as
    /// photoshop does
    pub tagged_layers: bool,
    /// the flattened picture, used when there are no layers
    pub merged: Vec<[f32; 4]>,
}

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

/// packbits, the way photoshop does it: runs of three or more repeat, the rest literal
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

fn encode_value(out: &mut Vec<u8>, v: f32, bits: u16) {
    match bits {
        8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
        16 => be16(out, (v.clamp(0.0, 1.0) * 65535.0).round() as u16),
        _ => out.extend_from_slice(&v.to_be_bytes()),
    }
}

fn predict(data: &mut [u8], w: usize, h: usize, bytes: usize) {
    let row = w * bytes;
    for y in 0..h {
        let line = &mut data[y * row..(y + 1) * row];
        match bytes {
            2 => {
                let vals: Vec<u16> = line
                    .as_chunks::<2>().0.iter()
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                let mut prev = 0u16;
                for (c, v) in line.as_chunks_mut::<2>().0.iter_mut().zip(vals) {
                    c.copy_from_slice(&v.wrapping_sub(prev).to_be_bytes());
                    prev = v;
                }
            }
            4 => {
                let orig = line.to_vec();
                for x in 0..w {
                    for b in 0..4 {
                        line[b * w + x] = orig[x * 4 + b];
                    }
                }
                for i in (1..line.len()).rev() {
                    line[i] = line[i].wrapping_sub(line[i - 1]);
                }
            }
            _ => {
                for i in (1..line.len()).rev() {
                    line[i] = line[i].wrapping_sub(line[i - 1]);
                }
            }
        }
    }
}

/// one channel: compression tag plus data
fn channel(values: &[f32], w: usize, h: usize, bits: u16, packing: Packing, big: bool) -> Vec<u8> {
    let mut raw = Vec::new();
    for v in values {
        encode_value(&mut raw, *v, bits);
    }
    let bytes = usize::from(bits / 8);
    let mut out = Vec::new();
    be16(&mut out, packing as u16);
    match packing {
        Packing::Raw => out.extend_from_slice(&raw),
        Packing::Rle => {
            let rows: Vec<Vec<u8>> = raw
                .chunks(w.max(1) * bytes)
                .take(h)
                .map(pack_bits)
                .collect();
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
        Packing::Zip | Packing::ZipPredicted => {
            if packing == Packing::ZipPredicted {
                predict(&mut raw, w, h, bytes);
            }
            let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
            z.write_all(&raw).expect("zip");
            out.extend_from_slice(&z.finish().expect("zip"));
        }
    }
    out
}

fn pascal(out: &mut Vec<u8>, s: &str, pad: usize) {
    let bytes: Vec<u8> = s.bytes().take(255).collect();
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

fn layer_info(doc: &WDoc) -> Vec<u8> {
    let mut records = Vec::new();
    let mut channel_data = Vec::new();
    for l in &doc.layers {
        let (x, y, w, h) = l.rect;
        let (wu, hu) = (w as usize, h as usize);
        let mut chans: Vec<(i16, Vec<u8>)> = Vec::new();
        let pick = |k: usize| l.pixels.iter().map(|p| p[k]).collect::<Vec<f32>>();
        if doc.gray {
            chans.push((
                -1,
                channel(&pick(3), wu, hu, doc.bits, doc.packing, doc.big),
            ));
            chans.push((0, channel(&pick(0), wu, hu, doc.bits, doc.packing, doc.big)));
        } else {
            chans.push((
                -1,
                channel(&pick(3), wu, hu, doc.bits, doc.packing, doc.big),
            ));
            for k in 0..3 {
                chans.push((
                    k as i16,
                    channel(&pick(k), wu, hu, doc.bits, doc.packing, doc.big),
                ));
            }
        }
        if let Some(m) = &l.mask {
            let (_, _, mw, mh) = m.rect;
            chans.push((
                -2,
                channel(
                    &m.values,
                    mw as usize,
                    mh as usize,
                    doc.bits,
                    doc.packing,
                    doc.big,
                ),
            ));
        }
        bei32(&mut records, y);
        bei32(&mut records, x);
        bei32(&mut records, y + h as i32);
        bei32(&mut records, x + w as i32);
        be16(&mut records, chans.len() as u16);
        for (id, data) in &chans {
            records.extend_from_slice(&id.to_be_bytes());
            len(&mut records, doc.big, data.len() as u64);
        }
        records.extend_from_slice(b"8BIM");
        records.extend_from_slice(&l.blend);
        records.push(l.opacity);
        records.push(u8::from(l.clipped));
        records.push(if l.hidden { 2 } else { 0 });
        records.push(0);
        let mut extra = Vec::new();
        match &l.mask {
            Some(m) => {
                be32(&mut extra, 20);
                let (mx, my, mw, mh) = m.rect;
                bei32(&mut extra, my);
                bei32(&mut extra, mx);
                bei32(&mut extra, my + mh as i32);
                bei32(&mut extra, mx + mw as i32);
                extra.push(m.default);
                extra.push(if m.disabled { 2 } else { 0 });
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
        if let Some(f) = l.fill {
            block(&mut extra, b"iOpa", &[f, 0, 0, 0]);
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
        if l.text {
            // the real block holds the text engine data; the reader only needs to see it
            block(&mut extra, b"TySh", &[0, 1, 0, 0]);
        }
        be32(&mut records, extra.len() as u32);
        records.extend_from_slice(&extra);
        for (_, data) in chans {
            channel_data.extend_from_slice(&data);
        }
    }
    let mut info = Vec::new();
    be16(&mut info, doc.layers.len() as u16);
    info.extend_from_slice(&records);
    info.extend_from_slice(&channel_data);
    if info.len() % 2 == 1 {
        info.push(0);
    }
    info
}

pub fn write(doc: &WDoc) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    be16(&mut out, if doc.big { 2 } else { 1 });
    out.extend_from_slice(&[0; 6]);
    be16(&mut out, if doc.gray { 2 } else { 4 });
    be32(&mut out, doc.height);
    be32(&mut out, doc.width);
    be16(&mut out, doc.bits);
    be16(&mut out, if doc.gray { 1 } else { 3 });
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
    if let Some(r) = doc.resolution {
        let mut d = Vec::new();
        be32(&mut d, (r * 65536.0) as u32);
        be16(&mut d, 1);
        be16(&mut d, 1);
        be32(&mut d, (r * 65536.0) as u32);
        be16(&mut d, 1);
        be16(&mut d, 1);
        resource(1005, &d);
    }
    if !doc.guides.is_empty() {
        let mut d = Vec::new();
        be32(&mut d, 1);
        be32(&mut d, 576);
        be32(&mut d, 576);
        be32(&mut d, doc.guides.len() as u32);
        for (at, dir) in &doc.guides {
            bei32(&mut d, *at);
            d.push(*dir);
        }
        resource(1032, &d);
    }
    if let Some(icc) = &doc.icc {
        resource(1039, icc);
    }
    be32(&mut out, res.len() as u32);
    out.extend_from_slice(&res);

    let mut section = Vec::new();
    if doc.layers.is_empty() {
        len(&mut section, doc.big, 0);
        be32(&mut section, 0);
    } else if doc.tagged_layers {
        len(&mut section, doc.big, 0);
        be32(&mut section, 0);
        let key = if doc.bits == 32 { b"Lr32" } else { b"Lr16" };
        let info = layer_info(doc);
        section.extend_from_slice(b"8BIM");
        section.extend_from_slice(key);
        let padded = info.len().div_ceil(4) * 4;
        len(&mut section, doc.big, padded as u64);
        section.extend_from_slice(&info);
        section.resize(section.len() + padded - info.len(), 0);
    } else {
        let info = layer_info(doc);
        len(&mut section, doc.big, info.len() as u64);
        section.extend_from_slice(&info);
        be32(&mut section, 0);
    }
    len(&mut out, doc.big, section.len() as u64);
    out.extend_from_slice(&section);

    // the flattened picture, rle compressed
    let (w, h) = (doc.width as usize, doc.height as usize);
    let nch = if doc.gray { 2 } else { 4 };
    let merged: Vec<[f32; 4]> = if doc.merged.len() == w * h {
        doc.merged.clone()
    } else {
        vec![[1.0; 4]; w * h]
    };
    let order: Vec<usize> = if doc.gray {
        vec![0, 3]
    } else {
        vec![0, 1, 2, 3]
    };
    be16(&mut out, 1);
    let bytes = usize::from(doc.bits / 8);
    let mut rows = Vec::new();
    for &k in order.iter().take(nch) {
        let mut raw = Vec::new();
        for p in &merged {
            encode_value(&mut raw, p[k], doc.bits);
        }
        for row in raw.chunks(w * bytes) {
            rows.push(pack_bits(row));
        }
    }
    for r in &rows {
        if doc.big {
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
