//! layered photoshop files (psd and the large document format, psb) as image documents.
//!
//! the `psd` crate only reads 8 bit layers without masks or fill, so vellum reads the file
//! itself. it keeps layers and groups (with their blend mode, opacity, fill, visibility,
//! locks, colour label and clipping), layer masks, guides, the resolution and the colour
//! profile, at 8, 16 or 32 bits per channel, with raw, packbits and zip compressed
//! channels. text, shape and smart object layers come in as the pixels photoshop stored
//! for them, marked with where they came from (`origin`).
//!
//! the file is read front to back through a buffered reader, so a big psb is never held in
//! memory whole: only one layer's channels at a time.
//!
//! the layout follows adobe's "photoshop file formats specification".

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::ZlibDecoder;
use vellum_gpu::{BlendMode, IRect};

use crate::adjustments::Adjustment;
use crate::document::{Content, DocColor, Guide, ImageDocument, Label, Layer, LayerMask, Locks};
use crate::raster::{Depth, Raster, TILE};

/// the largest side a psb may have, from the specification
const MAX_SIDE: u32 = 300_000;

struct Reader<R> {
    inner: R,
    /// psb files widen some lengths to 64 bits
    big: bool,
}

type Res<T> = Result<T, String>;

fn damaged(what: &str) -> String {
    format!("The Photoshop file is damaged or cut short ({what}).")
}

impl<R: Read + Seek> Reader<R> {
    fn bytes(&mut self, n: usize) -> Res<Vec<u8>> {
        let mut buf = vec![0; n];
        self.inner
            .read_exact(&mut buf)
            .map_err(|_| damaged("it ends early"))?;
        Ok(buf)
    }

    fn array<const N: usize>(&mut self) -> Res<[u8; N]> {
        let mut buf = [0; N];
        self.inner
            .read_exact(&mut buf)
            .map_err(|_| damaged("it ends early"))?;
        Ok(buf)
    }

    fn u8(&mut self) -> Res<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn i16(&mut self) -> Res<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn i32(&mut self) -> Res<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Res<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// a length that is 64 bits in psb files
    fn len(&mut self) -> Res<u64> {
        if self.big {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    fn pos(&mut self) -> Res<u64> {
        self.inner
            .stream_position()
            .map_err(|e| format!("The Photoshop file could not be read: {e}"))
    }

    fn seek(&mut self, to: u64) -> Res<()> {
        self.inner
            .seek(SeekFrom::Start(to))
            .map(|_| ())
            .map_err(|e| format!("The Photoshop file could not be read: {e}"))
    }

    fn skip(&mut self, n: u64) -> Res<()> {
        let at = self.pos()?;
        self.seek(at + n)
    }

    /// a pascal string padded so the whole thing is a multiple of `pad` bytes
    fn pascal(&mut self, pad: u64) -> Res<String> {
        let n = self.u8()?;
        let text = self.bytes(usize::from(n))?;
        let used = 1 + u64::from(n);
        let rem = used % pad;
        if rem != 0 {
            self.skip(pad - rem)?;
        }
        // old names are in the system's code page; latin 1 is the closest guess
        Ok(text.iter().map(|&b| char::from(b)).collect())
    }
}

/// what one layer record says, before its pixels are read
#[derive(Debug, Default)]
struct Record {
    rect: IRect,
    channels: Vec<(i16, u64)>,
    blend: [u8; 4],
    opacity: u8,
    clipped: bool,
    flags: u8,
    name: String,
    mask: Option<MaskInfo>,
    /// from 'lsct' or 'lsdk': 1 or 2 opens a group (open or closed), 3 ends one
    section: Option<u32>,
    section_blend: Option<[u8; 4]>,
    fill: Option<u8>,
    label: Label,
    locks: Locks,
    origin: Option<String>,
    /// an adjustment layer vellum can keep as one
    adjustment: Option<crate::adjustments::Adjustment>,
}

#[derive(Debug, Clone, Copy)]
struct MaskInfo {
    rect: IRect,
    default: u8,
    disabled: bool,
}

fn blend_of(key: [u8; 4]) -> BlendMode {
    match &key {
        b"diss" => BlendMode::Dissolve,
        b"dark" => BlendMode::Darken,
        b"mul " => BlendMode::Multiply,
        b"idiv" => BlendMode::ColorBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"dkCl" => BlendMode::DarkerColor,
        b"lite" => BlendMode::Lighten,
        b"scrn" => BlendMode::Screen,
        b"div " => BlendMode::ColorDodge,
        b"lddg" => BlendMode::LinearDodge,
        b"lgCl" => BlendMode::LighterColor,
        b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"vLit" => BlendMode::VividLight,
        b"lLit" => BlendMode::LinearLight,
        b"pLit" => BlendMode::PinLight,
        b"hMix" => BlendMode::HardMix,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Color,
        b"lum " => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// additional layer information keys whose length is 64 bits in a psb
fn wide_key(key: &[u8; 4]) -> bool {
    matches!(
        key,
        b"LMsk"
            | b"Lr16"
            | b"Lr32"
            | b"Layr"
            | b"Mt16"
            | b"Mt32"
            | b"Mtrn"
            | b"Alph"
            | b"FMsk"
            | b"lnk2"
            | b"FEid"
            | b"FXid"
            | b"PxSD"
    )
}

fn rect_of(top: i32, left: i32, bottom: i32, right: i32) -> IRect {
    if bottom <= top || right <= left {
        return IRect::EMPTY;
    }
    IRect::from_corners(left, top, right, bottom)
}

/// the header facts every part of the reader needs
#[derive(Debug, Clone, Copy)]
struct Header {
    width: u32,
    height: u32,
    channels: u16,
    bits: u16,
    mode: u16,
}

impl Header {
    fn bytes_per_value(&self) -> usize {
        usize::from(self.bits / 8).max(1)
    }
}

/// opens a psd or psb file as a layered document
pub fn open(path: &Path) -> Res<ImageDocument> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("{} could not be opened: {e}", path.display()))?;
    read(BufReader::with_capacity(1 << 20, file))
}

/// reads a psd or psb from anything that can seek
pub fn read<R: Read + Seek>(inner: R) -> Res<ImageDocument> {
    let mut r = Reader { inner, big: false };
    if &r.array::<4>()? != b"8BPS" {
        return Err("This is not a Photoshop file.".into());
    }
    let version = r.u16()?;
    r.big = match version {
        1 => false,
        2 => true,
        _ => return Err("This Photoshop file is from a version Vellum does not know.".into()),
    };
    r.skip(6)?;
    let header = Header {
        channels: r.u16()?,
        height: r.u32()?,
        width: r.u32()?,
        bits: r.u16()?,
        mode: r.u16()?,
    };
    if header.width == 0
        || header.height == 0
        || header.width > MAX_SIDE
        || header.height > MAX_SIDE
    {
        return Err(damaged("its size makes no sense"));
    }
    let depth = match header.bits {
        8 => Depth::U8,
        16 => Depth::U16,
        32 => Depth::F32,
        _ => {
            return Err(
                "This Photoshop file is in Bitmap mode (1 bit). Vellum opens it flattened.".into(),
            );
        }
    };
    let gray = match header.mode {
        3 => false,
        1 => true,
        4 => {
            return Err(
                "This Photoshop file is in CMYK mode. Vellum opens it flattened, in RGB.".into(),
            );
        }
        9 => {
            return Err(
                "This Photoshop file is in Lab mode. Vellum opens it flattened, in RGB.".into(),
            );
        }
        _ => {
            return Err(
                "This Photoshop file uses a colour mode Vellum cannot keep layers for. It opens flattened."
                    .into(),
            );
        }
    };

    // colour mode data, only used by indexed and duotone files
    let n = r.u32()?;
    r.skip(u64::from(n))?;

    let mut doc = ImageDocument::empty("", header.width, header.height, depth);
    read_resources(&mut r, &mut doc)?;

    let section_len = r.len()?;
    let section_start = r.pos()?;
    let section_end = section_start + section_len;
    let mut records = Vec::new();
    let mut layers_seen = false;
    if section_len > 0 {
        let info_len = r.len()?;
        let info_start = r.pos()?;
        if info_len > 0 {
            records = read_layer_info(&mut r, &header, depth, gray)?;
            layers_seen = true;
        }
        r.seek(info_start + info_len)?;
        // global layer mask info
        let n = r.u32()?;
        r.skip(u64::from(n))?;
        // 16 and 32 bit documents keep their layers in tagged blocks here instead
        while !layers_seen && r.pos()? + 12 <= section_end {
            let sig = r.array::<4>()?;
            if &sig != b"8BIM" && &sig != b"8B64" {
                break;
            }
            let key = r.array::<4>()?;
            let len = if r.big && wide_key(&key) {
                r.u64()?
            } else {
                u64::from(r.u32()?)
            };
            let start = r.pos()?;
            if matches!(&key, b"Lr16" | b"Lr32" | b"Layr") && len > 0 {
                // the block is the layer info without its own length, though some
                // writers keep the length too; a length matches the block's size
                let peek = r.len()?;
                if !(peek + 8 >= len && peek <= len) {
                    r.seek(start)?;
                }
                records = read_layer_info(&mut r, &header, depth, gray)?;
                layers_seen = true;
            }
            // blocks are padded to four bytes
            r.seek(start + len.div_ceil(4) * 4)?;
        }
    }
    r.seek(section_end)?;

    if records.is_empty() {
        // a flattened file: the merged picture becomes one layer
        let raster = read_merged(&mut r, &header, depth, gray)?;
        let id = doc.new_id();
        doc.layers.push(Layer::pixels(id, "Background", raster));
        doc.active = Some(id);
        return Ok(doc);
    }
    doc.layers = build_tree(&mut doc, records)?;
    doc.active = doc.layers.last().map(|l| l.id);
    Ok(doc)
}

fn read_resources<R: Read + Seek>(r: &mut Reader<R>, doc: &mut ImageDocument) -> Res<()> {
    let len = u64::from(r.u32()?);
    let start = r.pos()?;
    let end = start + len;
    while r.pos()? + 12 <= end {
        if &r.array::<4>()? != b"8BIM" {
            break;
        }
        let id = r.u16()?;
        r.pascal(2)?;
        let size = u64::from(r.u32()?);
        let at = r.pos()?;
        match id {
            // icc profile
            1039 if size > 0 => {
                let bytes = r.bytes(size as usize)?;
                doc.color = DocColor::from_icc(bytes);
            }
            // grid and guides, positions in 1/32 of a pixel
            1032 if size >= 16 => {
                let _version = r.u32()?;
                r.skip(8)?;
                let count = r.u32()?;
                for i in 0..count.min(10_000) {
                    if r.pos()? + 5 > at + size {
                        break;
                    }
                    let location = r.i32()?;
                    let direction = r.u8()?;
                    doc.guides.push(Guide {
                        id: i + 1,
                        vertical: direction == 0,
                        position: location as f32 / 32.0,
                    });
                }
            }
            // resolution, 16.16 fixed point pixels per inch
            1005 if size >= 4 => {
                let res = r.u32()? as f32 / 65536.0;
                if res.is_finite() && res > 0.0 {
                    doc.resolution = res;
                }
            }
            _ => {}
        }
        r.seek(at + size.div_ceil(2) * 2)?;
    }
    r.seek(end)
}

/// a layer's record and its decoded pixels
struct Decoded {
    record: Record,
    pixels: Raster,
    mask: Option<Raster>,
}

fn read_layer_info<R: Read + Seek>(
    r: &mut Reader<R>,
    header: &Header,
    depth: Depth,
    gray: bool,
) -> Res<Vec<Decoded>> {
    let count = r.i16()?.unsigned_abs();
    let mut records = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        records.push(read_record(r)?);
    }
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let (pixels, mask) = read_channels(r, header, &record, depth, gray)?;
        out.push(Decoded {
            record,
            pixels,
            mask,
        });
    }
    Ok(out)
}

fn read_record<R: Read + Seek>(r: &mut Reader<R>) -> Res<Record> {
    let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
    let mut rec = Record {
        rect: rect_of(top, left, bottom, right),
        ..Record::default()
    };
    let nch = r.u16()?;
    if nch > 64 {
        return Err(damaged("a layer has too many channels"));
    }
    for _ in 0..nch {
        let id = r.i16()?;
        let len = r.len()?;
        rec.channels.push((id, len));
    }
    if &r.array::<4>()? != b"8BIM" {
        return Err(damaged("a layer record is out of place"));
    }
    rec.blend = r.array()?;
    rec.opacity = r.u8()?;
    rec.clipped = r.u8()? == 1;
    rec.flags = r.u8()?;
    r.u8()?;
    let extra = u64::from(r.u32()?);
    let extra_start = r.pos()?;
    let extra_end = extra_start + extra;

    let mask_len = u64::from(r.u32()?);
    let mask_start = r.pos()?;
    if mask_len >= 18 {
        let (t, l, b, rt) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
        let default = r.u8()?;
        let flags = r.u8()?;
        rec.mask = Some(MaskInfo {
            rect: rect_of(t, l, b, rt),
            default,
            disabled: flags & 2 != 0,
        });
    }
    r.seek(mask_start + mask_len)?;
    let ranges = u64::from(r.u32()?);
    r.skip(ranges)?;
    rec.name = r.pascal(4)?;

    while r.pos()? + 12 <= extra_end {
        let sig = r.array::<4>()?;
        if &sig != b"8BIM" && &sig != b"8B64" {
            break;
        }
        let key = r.array::<4>()?;
        let len = if r.big && wide_key(&key) {
            r.u64()?
        } else {
            u64::from(r.u32()?)
        };
        let start = r.pos()?;
        if start + len > extra_end {
            break;
        }
        match &key {
            b"luni" if len >= 4 => {
                let chars = r.u32()? as usize;
                if chars * 2 + 4 <= len as usize {
                    let raw = r.bytes(chars * 2)?;
                    let units: Vec<u16> = raw
                        .as_chunks::<2>().0.iter()
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    let name = String::from_utf16_lossy(&units);
                    rec.name = name.trim_end_matches('\0').to_owned();
                }
            }
            b"lsct" | b"lsdk" if len >= 4 => {
                rec.section = Some(r.u32()?);
                if len >= 12 && &r.array::<4>()? == b"8BIM" {
                    rec.section_blend = Some(r.array()?);
                }
            }
            b"iOpa" if len >= 1 => rec.fill = Some(r.u8()?),
            b"lclr" if len >= 2 => {
                rec.label = match r.u16()? {
                    1 => Label::Red,
                    2 => Label::Orange,
                    3 => Label::Yellow,
                    4 => Label::Green,
                    5 => Label::Blue,
                    6 => Label::Violet,
                    7 => Label::Gray,
                    _ => Label::None,
                };
            }
            b"lspf" if len >= 4 => {
                let f = r.u32()?;
                rec.locks = Locks {
                    transparency: f & 1 != 0,
                    pixels: f & 2 != 0,
                    position: f & 4 != 0,
                    all: f & 0x8000_0000 != 0,
                };
            }
            b"nvrt" => rec.adjustment = Some(Adjustment::new("effect.invert")),
            b"post" if len >= 2 => {
                let levels = i32::from(r.u16()?.clamp(2, 255));
                rec.adjustment = Some(Adjustment::new("effect.posterize").with("levels", levels));
            }
            b"thrs" if len >= 2 => {
                let level = f32::from(r.u16()?.clamp(1, 255));
                rec.adjustment = Some(Adjustment::new("effect.threshold").with("level", level));
            }
            b"levl" if len >= 2 + 4 * 10 => {
                let _version = r.u16()?;
                // the composite record and the three channel records; levels here keeps one
                // channel, so the first that changes anything wins
                let mut chosen = None;
                for (i, channel) in ["rgb", "red", "green", "blue"].into_iter().enumerate() {
                    let v = [r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?];
                    if chosen.is_none()
                        && (v[0] != 0 || v[1] != 255 || v[2] != 0 || v[3] != 255 || v[4] != 100)
                    {
                        chosen = Some((i, channel, v));
                    }
                }
                let mut a = Adjustment::new("effect.levels");
                if let Some((_, channel, v)) = chosen {
                    a = a
                        .with("channel", channel)
                        .with("input_black", f32::from(v[0]))
                        .with("input_white", f32::from(v[1]))
                        .with("output_black", f32::from(v[2]))
                        .with("output_white", f32::from(v[3]))
                        .with("gamma", f32::from(v[4]) / 100.0);
                }
                rec.adjustment = Some(a);
            }
            b"TySh" | b"tySh" => rec.origin = Some("text".into()),
            b"SoLd" | b"SoLE" | b"PlLd" => {
                rec.origin.get_or_insert_with(|| "smart object".into());
            }
            b"vmsk" | b"vsms" | b"vogk" => {
                rec.origin.get_or_insert_with(|| "shape".into());
            }
            _ => {}
        }
        // most writers pad blocks to two bytes, photoshop to four; the next signature says
        let even = start + len.div_ceil(2) * 2;
        r.seek(even)?;
        if even + 4 <= extra_end && even % 4 != 0 {
            let peek = r.array::<4>()?;
            r.seek(even)?;
            if &peek != b"8BIM" && &peek != b"8B64" {
                r.seek(start + len.div_ceil(4) * 4)?;
            }
        }
    }
    r.seek(extra_end)?;
    Ok(rec)
}

/// undoes packbits on one row
fn unpack_bits(src: &[u8], out: &mut Vec<u8>, want: usize) -> Res<()> {
    let mut i = 0;
    while i < src.len() && out.len() < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let end = (i + count).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        } else if n != -128 {
            let count = (1 - i32::from(n)) as usize;
            let v = *src
                .get(i)
                .ok_or_else(|| damaged("a compressed row ends early"))?;
            i += 1;
            out.extend(std::iter::repeat_n(v, count));
        }
    }
    out.resize(want, 0);
    Ok(())
}

/// undoes photoshop's zip prediction: deltas along each row, and for 32 bit values the
/// four bytes of each value split into planes first
fn unpredict(data: &mut [u8], w: usize, h: usize, bytes: usize) {
    let row = w * bytes;
    for y in 0..h {
        let line = &mut data[y * row..(y + 1) * row];
        match bytes {
            2 => {
                let mut prev = 0u16;
                for c in line.as_chunks_mut::<2>().0.iter_mut() {
                    let v = u16::from_be_bytes([c[0], c[1]]).wrapping_add(prev);
                    c.copy_from_slice(&v.to_be_bytes());
                    prev = v;
                }
            }
            4 => {
                for i in 1..line.len() {
                    line[i] = line[i].wrapping_add(line[i - 1]);
                }
                let planes = line.to_vec();
                for x in 0..w {
                    for b in 0..4 {
                        line[x * 4 + b] = planes[b * w + x];
                    }
                }
            }
            _ => {
                for i in 1..line.len() {
                    line[i] = line[i].wrapping_add(line[i - 1]);
                }
            }
        }
    }
}

/// one channel's values for a `w` by `h` area, big endian as stored
fn read_channel<R: Read + Seek>(
    r: &mut Reader<R>,
    len: u64,
    w: usize,
    h: usize,
    bytes: usize,
) -> Res<Vec<u8>> {
    let start = r.pos()?;
    let size = w * h * bytes;
    if len < 2 {
        r.seek(start + len)?;
        return Ok(vec![0; size]);
    }
    let compression = r.u16()?;
    let data = if size == 0 {
        Vec::new()
    } else {
        decode_planar(r, compression, w, h, bytes, len - 2)?
    };
    r.seek(start + len)?;
    Ok(data)
}

fn decode_planar<R: Read + Seek>(
    r: &mut Reader<R>,
    compression: u16,
    w: usize,
    h: usize,
    bytes: usize,
    avail: u64,
) -> Res<Vec<u8>> {
    let size = w * h * bytes;
    match compression {
        0 => r.bytes(size),
        1 => {
            let mut counts = Vec::with_capacity(h);
            for _ in 0..h {
                counts.push(if r.big {
                    r.u32()? as usize
                } else {
                    usize::from(r.u16()?)
                });
            }
            let mut out = Vec::with_capacity(size);
            let row = w * bytes;
            for c in counts {
                let packed = r.bytes(c)?;
                let want = out.len() + row;
                unpack_bits(&packed, &mut out, want)?;
            }
            Ok(out)
        }
        2 | 3 => {
            let packed =
                r.bytes(usize::try_from(avail).map_err(|_| damaged("a channel is too big"))?)?;
            let mut out = Vec::with_capacity(size);
            ZlibDecoder::new(&packed[..])
                .take(size as u64)
                .read_to_end(&mut out)
                .map_err(|_| damaged("a zip compressed channel"))?;
            out.resize(size, 0);
            if compression == 3 {
                unpredict(&mut out, w, h, bytes);
            }
            Ok(out)
        }
        _ => Err(damaged("a channel uses an unknown compression")),
    }
}

/// a stored value as 0 to 1 (or any float for 32 bit)
#[inline]
fn value(data: &[u8], i: usize, bytes: usize) -> f32 {
    match bytes {
        1 => f32::from(data[i]) / 255.0,
        2 => f32::from(u16::from_be_bytes([data[i * 2], data[i * 2 + 1]])) / 65535.0,
        _ => f32::from_be_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]),
    }
}

/// writes planar channels into a colour raster a band of tiles at a time
fn planes_to_raster(rect: IRect, planes: [Option<&[u8]>; 4], bytes: usize, depth: Depth) -> Raster {
    let mut raster = Raster::rgba(depth);
    let w = rect.w as usize;
    let mut y = rect.y;
    while y < rect.bottom() {
        // bands end on tile edges so each tile is written once
        let next = ((y.div_euclid(TILE as i32) + 1) * TILE as i32).min(rect.bottom());
        let band = IRect::new(rect.x, y, rect.w, (next - y) as u32);
        let mut values = Vec::with_capacity(band.area() as usize * 4);
        for by in 0..band.h as usize {
            let row = (y - rect.y) as usize + by;
            for x in 0..w {
                let i = row * w + x;
                for (k, plane) in planes.iter().enumerate() {
                    values.push(match plane {
                        Some(p) => value(p, i, bytes),
                        None if k == 3 => 1.0,
                        None => 0.0,
                    });
                }
            }
        }
        raster.write(band, &values);
        y = next;
    }
    raster
}

fn read_channels<R: Read + Seek>(
    r: &mut Reader<R>,
    header: &Header,
    rec: &Record,
    depth: Depth,
    gray: bool,
) -> Res<(Raster, Option<Raster>)> {
    let bytes = header.bytes_per_value();
    let (w, h) = (rec.rect.w as usize, rec.rect.h as usize);
    let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut mask_plane = None;
    for &(id, len) in &rec.channels {
        match id {
            -1..=2 => {
                let data = read_channel(r, len, w, h, bytes)?;
                let slot = if id == -1 { 3 } else { id as usize };
                if gray && id == 0 {
                    planes[1] = Some(data.clone());
                    planes[2] = Some(data.clone());
                }
                planes[slot] = Some(data);
            }
            -2 => {
                let m = rec.mask.map(|m| m.rect).unwrap_or(IRect::EMPTY);
                mask_plane = Some(read_channel(r, len, m.w as usize, m.h as usize, bytes)?);
            }
            _ => r.skip(len)?,
        }
    }
    let pixels = if rec.rect.is_empty() || planes[0].is_none() {
        Raster::rgba(depth)
    } else {
        let refs = [
            planes[0].as_deref(),
            planes[1].as_deref(),
            planes[2].as_deref(),
            planes[3].as_deref(),
        ];
        planes_to_raster(rec.rect, refs, bytes, depth)
    };
    let mask = match (rec.mask, mask_plane) {
        (Some(info), Some(data)) => {
            let fill = f32::from(info.default) / 255.0;
            let mut m = Raster::gray(depth, fill);
            if !info.rect.is_empty() {
                let values: Vec<f32> = (0..info.rect.area() as usize)
                    .map(|i| value(&data, i, bytes).clamp(0.0, 1.0))
                    .collect();
                m.write(info.rect, &values);
            }
            Some(m)
        }
        _ => None,
    };
    Ok((pixels, mask))
}

/// the flattened picture after the layers, for files saved without layers
fn read_merged<R: Read + Seek>(
    r: &mut Reader<R>,
    header: &Header,
    depth: Depth,
    gray: bool,
) -> Res<Raster> {
    let compression = r.u16()?;
    let (w, h) = (header.width as usize, header.height as usize);
    let bytes = header.bytes_per_value();
    let n = usize::from(header.channels).min(if gray { 2 } else { 4 });
    let mut planes: Vec<Vec<u8>> = Vec::new();
    match compression {
        1 => {
            // every channel's row counts come first, then every channel's rows
            let total = h * usize::from(header.channels);
            let mut counts = Vec::with_capacity(total);
            for _ in 0..total {
                counts.push(if r.big {
                    r.u32()? as usize
                } else {
                    usize::from(r.u16()?)
                });
            }
            let row = w * bytes;
            for ch in 0..n {
                let mut out = Vec::with_capacity(w * h * bytes);
                for y in 0..h {
                    let packed = r.bytes(counts[ch * h + y])?;
                    let want = out.len() + row;
                    unpack_bits(&packed, &mut out, want)?;
                }
                planes.push(out);
            }
        }
        0 => {
            for _ in 0..n {
                planes.push(r.bytes(w * h * bytes)?);
            }
        }
        _ => {
            // zip for the merged picture is rare; it is one stream for every channel
            let mut rest = Vec::new();
            r.inner
                .read_to_end(&mut rest)
                .map_err(|_| damaged("the flattened picture"))?;
            let mut out = Vec::new();
            ZlibDecoder::new(&rest[..])
                .read_to_end(&mut out)
                .map_err(|_| damaged("the flattened picture"))?;
            let plane = w * h * bytes;
            out.resize(plane * usize::from(header.channels), 0);
            for ch in 0..n {
                let mut p = out[ch * plane..(ch + 1) * plane].to_vec();
                if compression == 3 {
                    unpredict(&mut p, w, h, bytes);
                }
                planes.push(p);
            }
        }
    }
    let get = |k: usize| planes.get(k).map(Vec::as_slice);
    let refs = if gray {
        [get(0), get(0), get(0), get(1)]
    } else {
        [get(0), get(1), get(2), get(3)]
    };
    Ok(planes_to_raster(
        IRect::of_size(header.width, header.height),
        refs,
        bytes,
        depth,
    ))
}

/// turns the bottom to top list of records into layers and groups. a group's children sit
/// between an end marker (section 3, below them) and the group's own record above them.
fn build_tree(doc: &mut ImageDocument, reads: Vec<Decoded>) -> Res<Vec<Layer>> {
    let mut stack: Vec<Vec<Layer>> = vec![Vec::new()];
    for read in reads {
        let rec = &read.record;
        match rec.section {
            Some(3) => stack.push(Vec::new()),
            Some(kind @ (1 | 2)) => {
                let children = if stack.len() > 1 {
                    stack.pop().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let id = doc.new_id();
                let mut group = Layer::group(id, rec.name.clone(), children);
                let key = rec.section_blend.unwrap_or(rec.blend);
                if let Content::Group {
                    expanded,
                    pass_through,
                    ..
                } = &mut group.content
                {
                    *expanded = kind == 1;
                    *pass_through = &key == b"pass" || &rec.blend == b"pass";
                }
                group.blend = blend_of(key);
                apply_common(&mut group, rec, read.mask);
                if let Some(top) = stack.last_mut() {
                    top.push(group);
                }
            }
            _ => {
                let id = doc.new_id();
                let mut layer = Layer::pixels(id, rec.name.clone(), read.pixels);
                if let Some(a) = &rec.adjustment {
                    layer.content = Content::Adjustment(a.clone());
                }
                layer.blend = blend_of(rec.blend);
                layer.origin = rec.origin.clone();
                apply_common(&mut layer, rec, read.mask);
                if let Some(top) = stack.last_mut() {
                    top.push(layer);
                }
            }
        }
    }
    // a file with an unclosed group still opens, its layers at the top level
    while stack.len() > 1 {
        let inner = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.extend(inner);
        }
    }
    Ok(stack.pop().unwrap_or_default())
}

fn apply_common(layer: &mut Layer, rec: &Record, mask: Option<Raster>) {
    layer.opacity = f32::from(rec.opacity) / 255.0;
    layer.fill = rec.fill.map_or(1.0, |f| f32::from(f) / 255.0);
    layer.visible = rec.flags & 2 == 0;
    layer.clipped = rec.clipped;
    layer.label = rec.label;
    layer.locks = rec.locks;
    if rec.flags & 1 != 0 {
        layer.locks.transparency = true;
    }
    if let (Some(info), Some(raster)) = (rec.mask, mask) {
        layer.mask = Some(LayerMask {
            raster,
            enabled: !info.disabled,
            linked: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_round_trips_runs_and_literals() {
        // a run of five 7s, then 1 2 3 as literals
        let packed = [0xFC, 7, 2, 1, 2, 3];
        let mut out = Vec::new();
        unpack_bits(&packed, &mut out, 8).unwrap();
        assert_eq!(out, [7, 7, 7, 7, 7, 1, 2, 3]);
    }

    #[test]
    fn prediction_undoes_deltas_and_planes() {
        // 16 bit: deltas of 1000 along a row
        let mut d = Vec::new();
        for _ in 0..3 {
            d.extend_from_slice(&1000u16.to_be_bytes());
        }
        unpredict(&mut d, 3, 1, 2);
        assert_eq!(u16::from_be_bytes([d[4], d[5]]), 3000);
        // 32 bit: one value 1.5 split into planes then byte deltas
        let v = 1.5f32.to_be_bytes();
        let mut row = vec![
            v[0],
            v[1].wrapping_sub(v[0]),
            v[2].wrapping_sub(v[1]),
            v[3].wrapping_sub(v[2]),
        ];
        unpredict(&mut row, 1, 1, 4);
        assert_eq!(f32::from_be_bytes([row[0], row[1], row[2], row[3]]), 1.5);
    }

    #[test]
    fn not_a_psd_says_so() {
        let err = read(std::io::Cursor::new(b"GIF89a....".to_vec())).unwrap_err();
        assert!(err.contains("not a Photoshop file"));
    }
}
