//! photoshop blend modes, the cpu reference. `shaders/blend.wgsl` mirrors this file line
//! for line, and the golden tests check the two agree.
//!
//! formulas follow adobe's published definitions (the pdf blend mode spec, which photoshop
//! shares) with photoshop's own choices where they differ: soft light uses a square root
//! for the lightening half, hard mix thresholds the sum of the two colours, and darker and
//! lighter colour compare the sum of the channels, as photoshop's help describes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BlendMode {
    #[default]
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// every mode in photoshop's menu order
    pub const ALL: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// the id the wgsl side switches on
    pub fn id(self) -> u32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }

    pub fn from_id(id: u32) -> Self {
        Self::ALL.get(id as usize).copied().unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Dissolve => "Dissolve",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::LinearBurn => "Linear Burn",
            BlendMode::DarkerColor => "Darker Color",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::LinearDodge => "Linear Dodge (Add)",
            BlendMode::LighterColor => "Lighter Color",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::HardLight => "Hard Light",
            BlendMode::VividLight => "Vivid Light",
            BlendMode::LinearLight => "Linear Light",
            BlendMode::PinLight => "Pin Light",
            BlendMode::HardMix => "Hard Mix",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
        }
    }

    /// a stable lowercase name, used for golden file names and the psd mapping
    pub fn slug(self) -> String {
        let label = self.label().to_lowercase();
        label
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_")
    }

    pub fn is_separable(self) -> bool {
        !matches!(
            self,
            BlendMode::DarkerColor
                | BlendMode::LighterColor
                | BlendMode::Hue
                | BlendMode::Saturation
                | BlendMode::Color
                | BlendMode::Luminosity
        )
    }
}

/// one channel of a separable mode. `b` is the backdrop, `s` the layer on top.
pub fn blend_channel(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve => s,
        BlendMode::Darken => b.min(s),
        BlendMode::Multiply => b * s,
        BlendMode::ColorBurn => color_burn(b, s),
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::Lighten => b.max(s),
        BlendMode::Screen => b + s - b * s,
        BlendMode::ColorDodge => color_dodge(b, s),
        BlendMode::LinearDodge => b + s,
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                b + (2.0 * s - 1.0) * (b.max(0.0).sqrt() - b)
            }
        }
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::VividLight => {
            if s <= 0.5 {
                color_burn(b, 2.0 * s)
            } else {
                color_dodge(b, 2.0 * (s - 0.5))
            }
        }
        BlendMode::LinearLight => b + 2.0 * s - 1.0,
        BlendMode::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        BlendMode::HardMix => {
            if b + s >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::Subtract => (b - s).max(0.0),
        BlendMode::Divide => {
            if s <= 0.0 {
                if b > 0.0 { 1.0 } else { 0.0 }
            } else {
                b / s
            }
        }
        // non separable modes are handled on whole colours
        _ => s,
    }
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        2.0 * b * s
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - s)
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 {
        1.0
    } else if s <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 {
        0.0
    } else if s >= 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

/// the luminosity used by the hue, saturation, colour and luminosity modes
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3], clamp_top: bool) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 && l - n > 1e-9 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if clamp_top && x > 1.0 && x - l > 1e-9 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

fn set_lum(c: [f32; 3], l: f32, clamp_top: bool) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d), clamp_top)
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max - min <= 1e-9 {
        return [0.0; 3];
    }
    c.map(|v| (v - min) * s / (max - min))
}

/// the blended colour of two straight (unpremultiplied) colours
pub fn blend_color(mode: BlendMode, b: [f32; 3], s: [f32; 3], clamp: bool) -> [f32; 3] {
    let out = match mode {
        BlendMode::DarkerColor => {
            if s[0] + s[1] + s[2] < b[0] + b[1] + b[2] {
                s
            } else {
                b
            }
        }
        BlendMode::LighterColor => {
            if s[0] + s[1] + s[2] > b[0] + b[1] + b[2] {
                s
            } else {
                b
            }
        }
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b), clamp),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b), clamp),
        BlendMode::Color => set_lum(s, lum(b), clamp),
        BlendMode::Luminosity => set_lum(b, lum(s), clamp),
        _ => [
            blend_channel(mode, b[0], s[0]),
            blend_channel(mode, b[1], s[1]),
            blend_channel(mode, b[2], s[2]),
        ],
    };
    if clamp {
        out.map(|v| v.clamp(0.0, 1.0))
    } else {
        out
    }
}

/// a small integer hash shared with wgsl, for dissolve
pub fn hash_u32(v: u32) -> u32 {
    let v = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

/// a stable random number in [0, 1) for a level 0 pixel
pub fn pixel_noise(x: i32, y: i32) -> f32 {
    hash_u32((x as u32) ^ hash_u32(y as u32)) as f32 / 4_294_967_296.0
}

/// composites a premultiplied layer pixel over a premultiplied backdrop pixel.
///
/// `alpha` scales the layer (opacity times fill times mask). `clamp` keeps colours in
/// [0, 1], which is right for gamma encoded working spaces; linear spaces keep values over
/// one so highlights survive. `x` and `y` are level 0 coordinates for dissolve.
pub fn composite(
    mode: BlendMode,
    backdrop: [f32; 4],
    layer: [f32; 4],
    alpha: f32,
    clamp: bool,
    x: i32,
    y: i32,
) -> [f32; 4] {
    let mut sa = layer[3] * alpha;
    if mode == BlendMode::Dissolve {
        sa = if pixel_noise(x, y) < sa { 1.0 } else { 0.0 };
    }
    if sa <= 0.0 {
        return backdrop;
    }
    let ba = backdrop[3];
    let cs = if layer[3] > 0.0 {
        [
            layer[0] / layer[3],
            layer[1] / layer[3],
            layer[2] / layer[3],
        ]
    } else {
        [0.0; 3]
    };
    let cb = if ba > 0.0 {
        [backdrop[0] / ba, backdrop[1] / ba, backdrop[2] / ba]
    } else {
        [0.0; 3]
    };
    let mixed = if ba > 0.0 {
        let b = blend_color(mode, cb, cs, clamp);
        [
            (1.0 - ba) * cs[0] + ba * b[0],
            (1.0 - ba) * cs[1] + ba * b[1],
            (1.0 - ba) * cs[2] + ba * b[2],
        ]
    } else {
        cs
    };
    [
        sa * mixed[0] + (1.0 - sa) * backdrop[0],
        sa * mixed[1] + (1.0 - sa) * backdrop[1],
        sa * mixed[2] + (1.0 - sa) * backdrop[2],
        sa + ba * (1.0 - sa),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(c: [f32; 3]) -> [f32; 4] {
        [c[0], c[1], c[2], 1.0]
    }

    fn blend(mode: BlendMode, b: f32, s: f32) -> f32 {
        composite(mode, opaque([b; 3]), opaque([s; 3]), 1.0, true, 0, 0)[0]
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn ids_round_trip_and_slugs_are_unique() {
        let mut slugs = std::collections::HashSet::new();
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_id(m.id()), m);
            assert!(slugs.insert(m.slug()));
        }
        assert_eq!(BlendMode::LinearDodge.slug(), "linear_dodge_add");
    }

    // hand worked values from adobe's formulas, with 8 bit style inputs
    #[test]
    fn separable_modes_match_hand_values() {
        let cases: &[(BlendMode, f32, f32, f32)] = &[
            (BlendMode::Normal, 0.2, 0.7, 0.7),
            (BlendMode::Multiply, 0.5, 0.5, 0.25),
            (BlendMode::Screen, 0.5, 0.5, 0.75),
            (BlendMode::Darken, 0.3, 0.6, 0.3),
            (BlendMode::Lighten, 0.3, 0.6, 0.6),
            (BlendMode::ColorBurn, 0.6, 0.5, 0.2),
            (BlendMode::ColorBurn, 1.0, 0.0, 1.0),
            (BlendMode::ColorBurn, 0.5, 0.0, 0.0),
            (BlendMode::LinearBurn, 0.6, 0.7, 0.3),
            (BlendMode::LinearBurn, 0.2, 0.3, 0.0),
            (BlendMode::ColorDodge, 0.4, 0.5, 0.8),
            (BlendMode::ColorDodge, 0.0, 1.0, 0.0),
            (BlendMode::ColorDodge, 0.4, 1.0, 1.0),
            (BlendMode::LinearDodge, 0.4, 0.3, 0.7),
            (BlendMode::LinearDodge, 0.8, 0.3, 1.0),
            (BlendMode::Overlay, 0.25, 0.6, 0.3),
            (BlendMode::Overlay, 0.75, 0.6, 0.8),
            (BlendMode::HardLight, 0.6, 0.25, 0.3),
            (BlendMode::HardLight, 0.6, 0.75, 0.8),
            (BlendMode::SoftLight, 0.25, 0.25, 0.15625),
            (BlendMode::SoftLight, 0.25, 0.75, 0.375),
            (BlendMode::VividLight, 0.6, 0.25, 0.2),
            (BlendMode::VividLight, 0.4, 0.75, 0.8),
            (BlendMode::LinearLight, 0.4, 0.6, 0.6),
            (BlendMode::PinLight, 0.6, 0.2, 0.4),
            (BlendMode::PinLight, 0.3, 0.8, 0.6),
            (BlendMode::HardMix, 0.4, 0.6, 1.0),
            (BlendMode::HardMix, 0.4, 0.5, 0.0),
            (BlendMode::Difference, 0.2, 0.7, 0.5),
            (BlendMode::Exclusion, 0.5, 0.5, 0.5),
            (BlendMode::Subtract, 0.7, 0.2, 0.5),
            (BlendMode::Subtract, 0.2, 0.7, 0.0),
            (BlendMode::Divide, 0.25, 0.5, 0.5),
            (BlendMode::Divide, 0.75, 0.5, 1.0),
        ];
        for &(mode, b, s, want) in cases {
            let got = blend(mode, b, s);
            assert!(near(got, want), "{mode:?} b={b} s={s}: {got} != {want}");
        }
    }

    #[test]
    fn component_modes_keep_the_right_parts() {
        let red = [0.8, 0.2, 0.2];
        let grey = [0.5, 0.5, 0.5];
        // luminosity of grey onto red keeps red's hue, takes grey's brightness
        let out = blend_color(BlendMode::Luminosity, red, grey, true);
        assert!(near(lum(out), 0.5));
        assert!(out[0] > out[1]);
        // colour of red onto grey keeps grey's brightness
        let out = blend_color(BlendMode::Color, grey, red, true);
        assert!(near(lum(out), 0.5));
        // saturation of grey removes all saturation
        let out = blend_color(BlendMode::Saturation, red, grey, true);
        assert!(sat(out) < 1e-6);
        // darker colour picks the darker whole colour
        assert_eq!(blend_color(BlendMode::DarkerColor, red, grey, true), red);
        assert_eq!(blend_color(BlendMode::LighterColor, red, grey, true), grey);
    }

    #[test]
    fn alpha_compositing_follows_source_over() {
        // half transparent white over opaque black is mid grey
        let out = composite(
            BlendMode::Normal,
            [0.0, 0.0, 0.0, 1.0],
            [0.5, 0.5, 0.5, 0.5],
            1.0,
            true,
            0,
            0,
        );
        assert!(near(out[0], 0.5) && near(out[3], 1.0));
        // anything over nothing is itself, whatever the mode
        for mode in BlendMode::ALL {
            if mode == BlendMode::Dissolve {
                continue;
            }
            let out = composite(mode, [0.0; 4], [0.3, 0.2, 0.1, 0.5], 1.0, true, 0, 0);
            assert!(near(out[0], 0.3) && near(out[3], 0.5), "{mode:?} {out:?}");
        }
        // opacity zero leaves the backdrop alone
        let b = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(
            composite(BlendMode::Multiply, b, [1.0; 4], 0.0, true, 0, 0),
            b
        );
    }

    #[test]
    fn dissolve_keeps_about_the_right_share() {
        let mut kept = 0;
        for y in 0..64 {
            for x in 0..64 {
                let out = composite(
                    BlendMode::Dissolve,
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0; 4],
                    0.3,
                    true,
                    x,
                    y,
                );
                if out[0] > 0.5 {
                    kept += 1;
                }
            }
        }
        let share = kept as f32 / 4096.0;
        assert!((share - 0.3).abs() < 0.03, "{share}");
    }
}
