//! exif metadata from photos, as [`MetadataTag`]s for the info panel.

use std::io::Cursor;

use exif::{Context, Exif, In, Reader, Tag, Value};

use crate::info::MetadataTag;

/// long values (maker notes, embedded previews) are cut to this many characters
const MAX_VALUE_CHARS: usize = 160;

/// what the exif block says about a photo
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExifSummary {
    pub tags: Vec<MetadataTag>,
    /// 1 to 8, 1 when missing
    pub orientation: u32,
    pub dpi: Option<f64>,
}

/// reads exif from a whole file (jpeg, tiff, heif, png, webp and tiff based raw files)
pub fn from_file_bytes(bytes: &[u8]) -> Option<ExifSummary> {
    let exif = Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()?;
    Some(summarise(&exif))
}

/// reads a raw exif block, as decoders hand it over
pub fn from_raw_block(block: &[u8]) -> Option<ExifSummary> {
    let exif = Reader::new().read_raw(block.to_vec()).ok()?;
    Some(summarise(&exif))
}

fn summarise(exif: &Exif) -> ExifSummary {
    let mut tags = Vec::new();
    for field in exif.fields() {
        // the thumbnail ifd repeats size fields for the tiny preview, which only confuses
        if field.ifd_num != In::PRIMARY || field.tag == Tag::MakerNote {
            continue;
        }
        let group = match field.tag.context() {
            Context::Gps => "GPS",
            _ => "EXIF",
        };
        let mut value = field.display_value().with_unit(exif).to_string();
        if value.chars().count() > MAX_VALUE_CHARS {
            value = value.chars().take(MAX_VALUE_CHARS).collect::<String>() + "...";
        }
        tags.push(MetadataTag::new(group, field.tag.to_string(), value));
    }
    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1);
    ExifSummary {
        tags,
        orientation,
        dpi: dpi(exif),
    }
}

fn dpi(exif: &Exif) -> Option<f64> {
    let field = exif.get_field(Tag::XResolution, In::PRIMARY)?;
    let Value::Rational(values) = &field.value else {
        return None;
    };
    let resolution = values.first()?.to_f64();
    // 2 is inches (the default), 3 is centimetres
    let unit = exif
        .get_field(Tag::ResolutionUnit, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(2);
    let dpi = if unit == 3 {
        resolution * 2.54
    } else {
        resolution
    };
    (dpi.is_finite() && dpi > 0.0).then_some(dpi)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// a little-endian tiff exif block holding the given (tag, type, count, value) entries,
    /// all values inline. used across this crate's tests.
    pub fn exif_block(entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let mut block = b"II*\0".to_vec();
        block.extend_from_slice(&8u32.to_le_bytes());
        block.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            block.extend_from_slice(&tag.to_le_bytes());
            block.extend_from_slice(&kind.to_le_bytes());
            block.extend_from_slice(&count.to_le_bytes());
            block.extend_from_slice(&value.to_le_bytes());
        }
        block.extend_from_slice(&0u32.to_le_bytes());
        block
    }

    #[test]
    fn orientation_and_tags_are_read_from_a_raw_block() {
        // orientation 6 (short), and an "Make" ascii tag of 3 bytes "Ab\0" stored inline
        let make = u32::from_le_bytes([b'A', b'b', 0, 0]);
        let block = exif_block(&[(0x010f, 2, 3, make), (0x0112, 3, 1, 6)]);
        let summary = from_raw_block(&block).unwrap();
        assert_eq!(summary.orientation, 6);
        let make = summary.tags.iter().find(|t| t.key == "Make").unwrap();
        assert!(make.value.contains("Ab"));
        assert!(make.sensitive);
    }

    #[test]
    fn missing_or_broken_exif_reads_as_nothing() {
        assert!(from_raw_block(b"nonsense").is_none());
        assert!(from_file_bytes(b"\x89PNG\r\n\x1a\n").is_none());
    }
}
