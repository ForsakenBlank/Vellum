//! colour and sampling helpers shared by the effects, the cpu side of what the prelude
//! gives every kernel. each function here has a wgsl twin of the same name in
//! `shaders/prelude.wgsl`, and the two must stay in step.

use crate::blend::pixel_noise;
use crate::graph::NodeCx;
use crate::image::CpuImage;

/// photoshop's luminosity weights, the same as the blend modes use
pub fn luma(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

pub fn clamp01(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| v.clamp(0.0, 1.0))
}

pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

pub fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        mix(a[0], b[0], t),
        mix(a[1], b[1], t),
        mix(a[2], b[2], t),
        mix(a[3], b[3], t),
    ]
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn rgb_to_hsl(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let l = (mx + mn) * 0.5;
    let d = mx - mn;
    if d <= 1e-7 {
        return [0.0, 0.0, l];
    }
    let s = if l < 0.5 {
        d / (mx + mn)
    } else {
        d / (2.0 - mx - mn).max(1e-7)
    };
    let h = if mx == c[0] {
        let h = (c[1] - c[2]) / d;
        if h < 0.0 { h + 6.0 } else { h }
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn hue_part(p: f32, q: f32, t: f32) -> f32 {
    let t = t - t.floor();
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

pub fn hsl_to_rgb(c: [f32; 3]) -> [f32; 3] {
    if c[1] <= 0.0 {
        return [c[2]; 3];
    }
    let q = if c[2] < 0.5 {
        c[2] * (1.0 + c[1])
    } else {
        c[2] + c[1] - c[2] * c[1]
    };
    let p = 2.0 * c[2] - q;
    [
        hue_part(p, q, c[0] + 1.0 / 3.0),
        hue_part(p, q, c[0]),
        hue_part(p, q, c[0] - 1.0 / 3.0),
    ]
}

/// a straight srgb colour (like a colour parameter) in the working space's encoding.
/// primaries are left alone: effect colours are picked by eye.
pub fn working_color(cx: &NodeCx, srgb: [f32; 4]) -> [f32; 3] {
    let enc = cx.working.encoding;
    [0, 1, 2].map(|k| enc.encode(crate::color::Transfer::Srgb.decode(srgb[k])))
}

/// bilinear sample at a position in level pixels, pixel centres at +0.5. the twin of
/// `sample0` in the prelude.
pub fn sample(img: &CpuImage, pos: [f32; 2], edge: bool) -> [f32; 4] {
    let fx = pos[0] - 0.5;
    let fy = pos[1] - 0.5;
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let get = |x: i32, y: i32| {
        if edge {
            img.get_clamped(x, y)
        } else {
            img.get(x, y)
        }
    };
    let (a, b, c, d) = (
        get(ix, iy),
        get(ix + 1, iy),
        get(ix, iy + 1),
        get(ix + 1, iy + 1),
    );
    mix4(mix4(a, b, tx), mix4(c, d, tx), ty)
}

/// smooth value noise in [0, 1], the twin of `value_noise` in the prelude
pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - x.floor(), y - y.floor());
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let s = seed.wrapping_mul(7919) as i32;
    let a = pixel_noise(ix.wrapping_add(s), iy);
    let b = pixel_noise(ix.wrapping_add(1).wrapping_add(s), iy);
    let c = pixel_noise(ix.wrapping_add(s), iy + 1);
    let d = pixel_noise(ix.wrapping_add(1).wrapping_add(s), iy + 1);
    mix(mix(a, b, sx), mix(c, d, sx), sy)
}

/// octaves of value noise, the twin of `fbm` in the prelude
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    let (mut fx, mut fy) = (x, y);
    for i in 0..octaves {
        sum += amp * value_noise(fx, fy, seed.wrapping_add(i));
        norm += amp;
        amp *= 0.5;
        fx *= 2.0;
        fy *= 2.0;
    }
    sum / f32::max(norm, 1e-6)
}

/// samples a table of `n` entries, the twin of `table_at` in the prelude. values past
/// either end carry on with a slope of one, so linear documents keep their highlights.
pub fn table_at(table: &[f32], base: usize, n: usize, v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x.floor() as usize).min(n - 2);
    let t = x - i as f32;
    let mut r = mix(table[base + i], table[base + i + 1], t);
    if v > 1.0 {
        r += v - 1.0;
    }
    if v < 0.0 {
        r += v;
    }
    r
}

/// photoshop's set luminosity, from the blend modes
pub fn set_lum(c: [f32; 3], l: f32, clamp_top: bool) -> [f32; 3] {
    let d = l - luma(c);
    clip_color(c.map(|v| v + d), clamp_top)
}

fn clip_color(c: [f32; 3], clamp_top: bool) -> [f32; 3] {
    let l = luma(c);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for c in [
            [0.8, 0.2, 0.3],
            [0.1, 0.9, 0.5],
            [0.3, 0.3, 0.3],
            [0.0, 0.0, 1.0],
        ] {
            let back = hsl_to_rgb(rgb_to_hsl(c));
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-5, "{c:?} {back:?}");
            }
        }
    }

    #[test]
    fn tables_carry_on_past_their_ends() {
        let t: Vec<f32> = (0..256).map(|i| i as f32 / 255.0).collect();
        assert!((table_at(&t, 0, 256, 0.5) - 0.5).abs() < 1e-6);
        assert!((table_at(&t, 0, 256, 1.5) - 1.5).abs() < 1e-6);
        assert!((table_at(&t, 0, 256, -0.25) + 0.25).abs() < 1e-6);
    }

    #[test]
    fn noise_stays_in_range_and_is_smooth() {
        let mut last = value_noise(0.0, 0.3, 4);
        for i in 1..200 {
            let v = value_noise(i as f32 * 0.01, 0.3, 4);
            assert!((0.0..=1.0).contains(&v));
            assert!((v - last).abs() < 0.05);
            last = v;
        }
        assert!((0.0..=1.0).contains(&fbm(3.3, 9.1, 5, 1)));
    }
}
