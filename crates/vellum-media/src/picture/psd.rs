//! photoshop files. for now only the flattened image photoshop saves next to the layers
//! ("maximise compatibility"), which is what every viewer shows. layers come in phase 6.

use std::path::Path;

use ::image::RgbaImage;

use super::{Picture, PictureFormat};
use crate::error::{MediaError, MediaResult};

pub fn decode(path: &Path) -> MediaResult<Picture> {
    let bytes = std::fs::read(path).map_err(|err| MediaError::io(path, err))?;
    let psd = ::psd::Psd::from_bytes(&bytes).map_err(|err| MediaError::decode(path, err))?;
    let (w, h) = (psd.width(), psd.height());
    let image = RgbaImage::from_raw(w, h, psd.rgba()).ok_or_else(|| {
        MediaError::decode(path, "the flattened image is missing or the wrong size")
    })?;
    Ok(Picture::still(PictureFormat::Psd, w, h, image))
}

/// width and height straight from the 26 byte header
pub fn dimensions(path: &Path) -> MediaResult<(u32, u32)> {
    use std::io::Read;
    let mut header = [0_u8; 26];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|err| MediaError::io(path, err))?;
    if &header[..4] != b"8BPS" {
        return Err(MediaError::decode(path, "it is not a Photoshop file"));
    }
    let h = u32::from_be_bytes([header[14], header[15], header[16], header[17]]);
    let w = u32::from_be_bytes([header[18], header[19], header[20], header[21]]);
    Ok((w, h))
}
