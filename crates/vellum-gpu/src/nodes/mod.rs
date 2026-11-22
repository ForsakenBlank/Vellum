//! the built in nodes. adding an effect is one file here plus one line in
//! [`register_builtins`]: the file declares its [`NodeInfo`](crate::graph::NodeInfo), a
//! wgsl kernel body and the matching cpu code. `levels.rs` is the simplest one to copy;
//! `gaussian_blur.rs` shows a spatial effect with two passes.

use rayon::prelude::*;

use crate::graph::{NodeRegistry, Out};
use crate::image::CpuImage;
use crate::params::ChoiceOption;
use crate::rect::IRect;

pub mod adjust;
pub mod blend;
pub mod blur;
pub mod camera_raw;
pub mod color_convert;
pub mod colorfn;
pub mod detail;
pub mod distort;
pub mod gaussian_blur;
pub mod gradient;
pub mod hue_saturation;
pub mod image;
pub mod levels;
pub mod mask;
pub mod output;
pub mod render;
pub mod sharpen;
pub mod simple;
pub mod solid;
pub mod stylize;
pub mod tone;
pub mod transform;

pub use blend::BlendNode;
pub use color_convert::{ColorConvertNode, LutNode};
pub use gaussian_blur::GaussianBlur;
pub use gradient::GradientNode;
pub use hue_saturation::HueSaturation;
pub use image::ImageNode;
pub use levels::Levels;
pub use mask::MaskNode;
pub use output::OutputNode;
pub use sharpen::Sharpen;
pub use solid::SolidNode;
pub use transform::TransformNode;

pub fn register_builtins(r: &mut NodeRegistry) {
    r.register(ImageNode);
    r.register(SolidNode);
    r.register(GradientNode);
    r.register(BlendNode);
    r.register(MaskNode);
    r.register(TransformNode);
    r.register(ColorConvertNode);
    r.register(LutNode);
    r.register(OutputNode);
    r.register(GaussianBlur);
    r.register(Levels);
    r.register(HueSaturation);
    r.register(Sharpen);
    r.register(adjust::Curves);
    r.register(adjust::BrightnessContrast);
    r.register(adjust::Invert);
    r.register(adjust::Posterize);
    r.register(adjust::Threshold);
    r.register(adjust::Exposure);
    r.register(adjust::Vibrance);
    r.register(adjust::ColorBalance);
    r.register(adjust::BlackWhite);
    r.register(adjust::PhotoFilter);
    r.register(adjust::ChannelMixer);
    r.register(adjust::GradientMap);
    r.register(adjust::SelectiveColor);
    r.register(adjust::MatchColor);
    r.register(adjust::ReplaceColor);
    r.register(tone::shadows_highlights());
    r.register(tone::hdr_toning());
    r.register(camera_raw::CameraRaw);
    r.register(blur::MotionBlur);
    r.register(blur::RadialBlur);
    r.register(blur::LensBlur);
    r.register(blur::FocusBlur);
    r.register(blur::SurfaceBlur);
    r.register(detail::smart_sharpen());
    r.register(detail::high_pass());
    r.register(detail::reduce_noise());
    r.register(detail::AddNoise);
    r.register(detail::FilmGrain);
    r.register(distort::Wave);
    r.register(distort::Ripple);
    r.register(distort::Twirl);
    r.register(distort::Polar);
    r.register(distort::Displace);
    r.register(distort::ChromaticAberration);
    r.register(distort::Glitch);
    r.register(stylize::Emboss);
    r.register(stylize::FindEdges);
    r.register(stylize::GlowingEdges);
    r.register(stylize::OilPaint);
    r.register(stylize::Mosaic);
    r.register(stylize::Halftone);
    r.register(render::Clouds);
    r.register(render::LensFlare);
    r.register(render::Lighting);
}

/// applies a per pixel function to an input over `region`, rows in parallel. pixels where
/// the input is transparent stay transparent unless `f` says otherwise.
pub fn map_pixels(
    input: &CpuImage,
    region: IRect,
    f: impl Fn([f32; 4], i32, i32) -> [f32; 4] + Sync,
) -> CpuImage {
    let region = region.intersect(&input.rect);
    CpuImage::from_fn(region, |x, y| f(input.get(x, y), x, y))
}

/// the usual answer when the only input is missing
pub fn empty_or<T>(input: Option<T>, f: impl FnOnce(T) -> Out<CpuImage>) -> Out<CpuImage> {
    match input {
        Some(i) => f(i),
        None => Out::Empty,
    }
}

pub fn unpremultiply(c: [f32; 4]) -> [f32; 4] {
    if c[3] <= 0.0 {
        return [0.0; 4];
    }
    [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
}

pub fn premultiply(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

/// runs `f` over rows of `out` in parallel, giving the row's y and its pixels
pub fn par_rows(out: &mut CpuImage, f: impl Fn(i32, &mut [[f32; 4]]) + Sync) {
    let rect = out.rect;
    if rect.is_empty() {
        return;
    }
    out.data
        .par_chunks_mut(rect.w as usize)
        .enumerate()
        .for_each(|(row, line)| f(rect.y + row as i32, line));
}

/// the colour spaces offered by colour nodes, in menu order
pub const SPACE_OPTIONS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "working",
        label: "Working space",
    },
    ChoiceOption {
        id: "srgb",
        label: "sRGB",
    },
    ChoiceOption {
        id: "linear_srgb",
        label: "Linear sRGB",
    },
    ChoiceOption {
        id: "rec709",
        label: "Rec.709",
    },
    ChoiceOption {
        id: "rec2020",
        label: "Rec.2020",
    },
    ChoiceOption {
        id: "rec2020_pq",
        label: "Rec.2020 PQ",
    },
    ChoiceOption {
        id: "rec2020_hlg",
        label: "Rec.2020 HLG",
    },
    ChoiceOption {
        id: "display_p3",
        label: "Display P3",
    },
    ChoiceOption {
        id: "adobe_rgb",
        label: "Adobe RGB",
    },
    ChoiceOption {
        id: "acescg",
        label: "ACEScg",
    },
    ChoiceOption {
        id: "acescct",
        label: "ACEScct",
    },
    ChoiceOption {
        id: "aces2065_1",
        label: "ACES2065-1",
    },
];

pub fn space_by_id(id: &str, working: crate::color::WorkingSpace) -> crate::color::ColorSpace {
    use crate::color::ColorSpace as C;
    match id {
        "srgb" => C::SRGB,
        "linear_srgb" => C::LINEAR_SRGB,
        "rec709" => C::REC709,
        "rec2020" => C::REC2020,
        "rec2020_pq" => C::REC2020_PQ,
        "rec2020_hlg" => C::REC2020_HLG,
        "display_p3" => C::DISPLAY_P3,
        "adobe_rgb" => C::ADOBE_RGB,
        "acescg" => C::ACESCG,
        "acescct" => C::ACESCCT,
        "aces2065_1" => C::ACES2065_1,
        _ => working.space(),
    }
}
