//! captions for gifs, drawn by resvg into a transparent picture that ffmpeg lays over the
//! frames. ffmpeg's own `drawtext` needs a build with freetype, which the lgpl build vellum
//! ships may not have, and drawing here keeps the text identical on every computer.
//!
//! the font is Inter Black, bundled (see `assets/OFL.txt`), so a caption never comes out
//! blank on a machine without Impact.

use std::sync::{Arc, OnceLock};

use image::RgbaImage;
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_media::{MediaError, MediaResult};

const FONT: &[u8] = include_bytes!("../../assets/Inter-Black.otf");
const FAMILY: &str = "Inter";
/// inter black runs a little narrower than this on average, so lines rarely overflow
const CHAR_WIDTH: f64 = 0.62;
const LINE_HEIGHT: f64 = 1.12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CaptionStyle {
    /// white capitals with a black outline over the picture, top and bottom
    #[default]
    Classic,
    /// black text on a white bar added above (and below) the picture
    Bar,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Caption {
    pub top: String,
    pub bottom: String,
    pub style: CaptionStyle,
    /// 1 is the normal size, 0.5 to 2 makes it smaller or bigger
    pub scale: f64,
}

impl Caption {
    pub fn is_empty(&self) -> bool {
        self.top.trim().is_empty() && self.bottom.trim().is_empty()
    }

    fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale.clamp(0.4, 2.5)
        } else {
            1.0
        }
    }
}

/// what to lay over the frames, and how much the bar style adds above and below
pub struct CaptionLayer {
    pub image: RgbaImage,
    /// pixels added above the picture, 0 for the classic style
    pub pad_top: u32,
    pub pad_bottom: u32,
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_font_data(FONT.to_vec());
            db.set_sans_serif_family(FAMILY);
            Arc::new(db)
        })
        .clone()
}

/// draws the caption for frames `width` by `height`. the result is the size of the frame
/// plus any bars.
pub fn render(caption: &Caption, width: u32, height: u32) -> MediaResult<CaptionLayer> {
    let w = f64::from(width);
    let h = f64::from(height);
    let mut body = String::new();
    let (pad_top, pad_bottom) = match caption.style {
        CaptionStyle::Classic => {
            let size = (w.min(h * 1.4) / 9.0 * caption.scale()).max(10.0);
            let stroke = (size / 12.0).max(1.0);
            let top = wrap(&caption.top.to_uppercase(), w * 0.94, size);
            let bottom = wrap(&caption.bottom.to_uppercase(), w * 0.94, size);
            let style = format!(
                "font-family=\"{FAMILY}\" font-weight=\"900\" fill=\"#fff\" stroke=\"#000\" stroke-width=\"{stroke:.1}\" stroke-linejoin=\"round\" paint-order=\"stroke\" text-anchor=\"middle\""
            );
            let first_baseline = size * 0.95 + h * 0.03;
            body.push_str(&text_block(&top, w / 2.0, first_baseline, size, &style));
            if !bottom.is_empty() {
                let lines = bottom.len() as f64;
                let baseline = h - h * 0.04 - size * 0.2 - (lines - 1.0) * size * LINE_HEIGHT;
                body.push_str(&text_block(&bottom, w / 2.0, baseline, size, &style));
            }
            (0, 0)
        }
        CaptionStyle::Bar => {
            let size = (w / 14.0 * caption.scale()).max(10.0);
            let style = format!(
                "font-family=\"{FAMILY}\" font-weight=\"900\" fill=\"#000\" text-anchor=\"middle\""
            );
            let top = wrap(&caption.top, w * 0.9, size);
            let bottom = wrap(&caption.bottom, w * 0.9, size);
            let bar = |lines: usize| -> f64 {
                if lines == 0 {
                    0.0
                } else {
                    // even heights keep yuv encoders happy
                    let raw = lines as f64 * size * LINE_HEIGHT + size * 1.1;
                    (raw / 2.0).ceil() * 2.0
                }
            };
            let top_bar = bar(top.len());
            let bottom_bar = bar(bottom.len());
            let total = h + top_bar + bottom_bar;
            if top_bar > 0.0 {
                body.push_str(&format!(
                    "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{top_bar}\" fill=\"#fff\"/>"
                ));
                let baseline = size * 0.55 + size * 0.9;
                body.push_str(&text_block(&top, w / 2.0, baseline, size, &style));
            }
            if bottom_bar > 0.0 {
                let y = total - bottom_bar;
                body.push_str(&format!(
                    "<rect x=\"0\" y=\"{y}\" width=\"{w}\" height=\"{bottom_bar}\" fill=\"#fff\"/>"
                ));
                let baseline = y + size * 0.55 + size * 0.9;
                body.push_str(&text_block(&bottom, w / 2.0, baseline, size, &style));
            }
            // whole pixels, the bars were rounded to even numbers above
            (top_bar as u32, bottom_bar as u32)
        }
    };
    let full_h = height + pad_top + pad_bottom;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{full_h}\" viewBox=\"0 0 {width} {full_h}\">{body}</svg>"
    );
    let options = usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options)
        .map_err(|err| MediaError::Unsupported(format!("The caption could not be drawn: {err}")))?;
    let mut pixmap = tiny_skia::Pixmap::new(width, full_h)
        .ok_or_else(|| MediaError::Unsupported("The caption has no size.".to_owned()))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|px| {
            let c = px.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let image = RgbaImage::from_raw(width, full_h, rgba)
        .ok_or_else(|| MediaError::Unsupported("The caption had the wrong size.".to_owned()))?;
    Ok(CaptionLayer {
        image,
        pad_top,
        pad_bottom,
    })
}

fn text_block(lines: &[String], x: f64, first_baseline: f64, size: f64, style: &str) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("<text font-size=\"{size:.1}\" {style}>");
    for (i, line) in lines.iter().enumerate() {
        let y = first_baseline + i as f64 * size * LINE_HEIGHT;
        out.push_str(&format!(
            "<tspan x=\"{x:.1}\" y=\"{y:.1}\">{}</tspan>",
            escape(line)
        ));
    }
    out.push_str("</text>");
    out
}

/// splits text into lines that fit `width` at font `size`, breaking between words, and
/// inside a word only when one word is wider than a line
pub fn wrap(text: &str, width: f64, size: f64) -> Vec<String> {
    let max = ((width / (size * CHAR_WIDTH)).floor() as usize).max(1);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            while word.len() > max {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                lines.push(word.drain(..max).collect());
            }
            let word: String = word.into_iter().collect();
            let needed =
                line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
            if needed > max && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_wraps_between_words() {
        let lines = wrap("when the code compiles on the first try", 200.0, 20.0);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 16));
        assert_eq!(lines.join(" "), "when the code compiles on the first try");
        // a word longer than a line is split rather than spilling off the picture
        let long = wrap("aaaaaaaaaaaaaaaaaaaaaaaaa", 100.0, 20.0);
        assert!(long.len() > 1);
        assert!(wrap("   ", 100.0, 20.0).is_empty());
    }

    #[test]
    fn classic_captions_draw_white_text_with_an_outline() {
        let caption = Caption {
            top: "top text".into(),
            bottom: "bottom <text> & more".into(),
            ..Caption::default()
        };
        let layer = render(&caption, 320, 240).unwrap();
        assert_eq!(layer.image.dimensions(), (320, 240));
        assert_eq!((layer.pad_top, layer.pad_bottom), (0, 0));
        let opaque = |y0: u32, y1: u32| {
            (y0..y1)
                .flat_map(|y| (0..320).map(move |x| (x, y)))
                .filter(|(x, y)| layer.image.get_pixel(*x, *y)[3] > 200)
                .count()
        };
        assert!(opaque(0, 60) > 200, "no text at the top");
        assert!(opaque(180, 240) > 200, "no text at the bottom");
        assert_eq!(opaque(100, 140), 0, "the middle should stay clear");
        let white = layer.image.pixels().any(|p| p.0 == [255, 255, 255, 255]);
        let black = layer.image.pixels().any(|p| p[0] < 30 && p[3] > 250);
        assert!(white && black);
    }

    #[test]
    fn bars_add_room_above_and_below() {
        let caption = Caption {
            top: "me explaining my gif".into(),
            bottom: "the gif".into(),
            style: CaptionStyle::Bar,
            scale: 1.0,
        };
        let layer = render(&caption, 300, 200).unwrap();
        assert!(layer.pad_top > 20 && layer.pad_top.is_multiple_of(2));
        assert!(layer.pad_bottom > 20);
        assert_eq!(layer.image.height(), 200 + layer.pad_top + layer.pad_bottom);
        // the bar is white where there is no text
        assert_eq!(layer.image.get_pixel(1, 1).0, [255, 255, 255, 255]);
        // the picture area stays see-through
        assert_eq!(layer.image.get_pixel(150, layer.pad_top + 100)[3], 0);
    }
}
