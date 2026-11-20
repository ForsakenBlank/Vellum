//! camera raw through rawler.
//!
//! previews use the jpeg the camera embeds in the file when it is big enough: it is what
//! the camera showed on its screen and takes milliseconds instead of a full develop. a full
//! decode demosaics and colour corrects the sensor data.

use std::path::Path;

use ::image::DynamicImage;
use ::image::metadata::Orientation;
use rawler::decoders::RawDecodeParams;
use rawler::imgop::develop::RawDevelop;
use rawler::rawsource::RawSource;

use super::{DecodeOptions, Picture, PictureFormat};
use crate::error::{MediaError, MediaResult};

pub fn decode(path: &Path, options: DecodeOptions) -> MediaResult<Picture> {
    let bad = |err: rawler::RawlerError| MediaError::decode(path, err);
    let source = RawSource::new(path).map_err(|err| MediaError::io(path, err))?;
    let decoder = rawler::get_decoder(&source).map_err(bad)?;
    let params = RawDecodeParams::default();
    let orientation = crate::probe::exif_orientation(path).unwrap_or(Orientation::NoTransforms);

    if let Some(size) = options.size {
        let embedded = [
            decoder.thumbnail_image(&source, &params),
            decoder.preview_image(&source, &params),
            decoder.full_image(&source, &params),
        ];
        let usable = embedded
            .into_iter()
            .filter_map(|found| found.ok().flatten())
            .find(|image| image.width().max(image.height()) >= size);
        if let Some(mut image) = usable {
            image.apply_orientation(orientation);
            let (w, h) = (image.width(), image.height());
            return Ok(Picture::still(PictureFormat::Raw, w, h, image.into_rgba8()));
        }
    }

    let raw = decoder.raw_image(&source, &params, false).map_err(bad)?;
    let developed = RawDevelop::default()
        .develop_intermediate(&raw)
        .map_err(bad)?
        .to_dynamic_image()
        .ok_or_else(|| MediaError::decode(path, "the developed image had the wrong size"))?;
    let mut image = DynamicImage::ImageRgba8(developed.into_rgba8());
    image.apply_orientation(orientation);
    let (w, h) = (image.width(), image.height());
    Ok(Picture::still(PictureFormat::Raw, w, h, image.into_rgba8()))
}

/// the sensor size, read without decompressing anything
pub fn dimensions(path: &Path) -> MediaResult<(u32, u32)> {
    let bad = |err: rawler::RawlerError| MediaError::decode(path, err);
    let source = RawSource::new(path).map_err(|err| MediaError::io(path, err))?;
    let decoder = rawler::get_decoder(&source).map_err(bad)?;
    let raw = decoder
        .raw_image(&source, &RawDecodeParams::default(), true)
        .map_err(bad)?;
    let (w, h) = match raw.crop_area {
        Some(crop) => (crop.d.w, crop.d.h),
        None => (raw.width, raw.height),
    };
    let orientation = crate::probe::exif_orientation(path).unwrap_or(Orientation::NoTransforms);
    let w = u32::try_from(w).unwrap_or(u32::MAX);
    let h = u32::try_from(h).unwrap_or(u32::MAX);
    Ok(super::upright(w, h, orientation))
}
