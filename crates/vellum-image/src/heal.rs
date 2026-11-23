//! healing and content aware fill.
//!
//! healing copies texture from a source and fixes its colour and brightness to match the
//! destination. the difference between destination and source is known around the area
//! being healed; inside it is filled in smoothly (a membrane, which is what a poisson
//! blend comes down to), so the copied texture takes on the destination's tones and the
//! seam disappears. the membrane is solved with a push-pull pyramid, then smoothed with a
//! few relaxation passes at full size.
//!
//! content aware fill picks the source itself: it tries offsets around the hole and keeps
//! the one whose surroundings match the hole's surroundings best, then heals from there.

use vellum_gpu::IRect;

use crate::raster::Raster;

/// the largest area healed in one go, to keep the dense buffers sane
const MAX_AREA: u64 = 4096 * 4096;

/// a dense block of straight rgba
#[derive(Clone)]
struct Block {
    rect: IRect,
    px: Vec<[f32; 4]>,
}

impl Block {
    fn read(r: &Raster, rect: IRect) -> Self {
        let v = r.read(rect);
        Self {
            rect,
            px: v
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect(),
        }
    }

    fn at(&self, x: i32, y: i32) -> [f32; 4] {
        let r = self.rect;
        let x = x.clamp(r.x, r.right() - 1);
        let y = y.clamp(r.y, r.bottom() - 1);
        self.px[((y - r.y) as u32 * r.w + (x - r.x) as u32) as usize]
    }
}

/// fills the unknown parts of `values` (where `known` is zero) smoothly from the known
/// parts, by pulling known values down a pyramid and pushing them back up
fn membrane(w: usize, h: usize, values: &mut [[f32; 4]], known: &[f32]) {
    if w == 0 || h == 0 {
        return;
    }
    // pull: weighted averages at each level
    let mut levels = vec![(w, h, values.to_vec(), known.to_vec())];
    loop {
        let (lw, lh, v, k) = match levels.last() {
            Some(l) if l.0 > 1 || l.1 > 1 => l.clone(),
            _ => break,
        };
        let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
        let mut nv = vec![[0.0f32; 4]; nw * nh];
        let mut nk = vec![0.0f32; nw * nh];
        for y in 0..lh {
            for x in 0..lw {
                let i = y * lw + x;
                let j = (y / 2) * nw + x / 2;
                for c in 0..4 {
                    nv[j][c] += v[i][c] * k[i];
                }
                nk[j] += k[i];
            }
        }
        for (v, k) in nv.iter_mut().zip(nk.iter_mut()) {
            if *k > 0.0 {
                for c in v.iter_mut() {
                    *c /= *k;
                }
                *k = k.min(1.0);
            }
        }
        levels.push((nw, nh, nv, nk));
    }
    // push: fill each level's gaps from the coarser one, blending by how known each is
    for li in (0..levels.len() - 1).rev() {
        let (cw, _, cv, _) = levels[li + 1].clone();
        let (lw, lh, v, k) = &mut levels[li];
        for y in 0..*lh {
            for x in 0..*lw {
                let i = y * *lw + x;
                let coarse = cv[(y / 2) * cw + x / 2];
                let t = k[i].clamp(0.0, 1.0);
                for c in 0..4 {
                    v[i][c] = v[i][c] * t + coarse[c] * (1.0 - t);
                }
            }
        }
    }
    let (_, _, filled, _) = levels.swap_remove(0);
    values.copy_from_slice(&filled);
    // relax: a few smoothing passes over the unknown part only
    for _ in 0..40 {
        let prev = values.to_vec();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if known[i] >= 1.0 {
                    continue;
                }
                let mut acc = [0.0f32; 4];
                let mut n = 0.0;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let p = prev[ny as usize * w + nx as usize];
                    for c in 0..4 {
                        acc[c] += p[c];
                    }
                    n += 1.0;
                }
                let t = known[i];
                for c in 0..4 {
                    values[i][c] = prev[i][c] * t + acc[c] / n * (1.0 - t);
                }
            }
        }
    }
}

/// heals `dest` over the area `coverage` covers (0 to 1, a grey raster) with texture read
/// from `source` at an offset of (dx, dy). returns the healed pixels.
pub fn heal(
    dest: &Raster,
    source: &Raster,
    dx: i32,
    dy: i32,
    coverage: &Raster,
    area: IRect,
) -> Result<Raster, String> {
    let rect = area.expand(3);
    if rect.is_empty() {
        return Ok(dest.clone());
    }
    if rect.area() > MAX_AREA {
        return Err("That area is too big to heal in one go. Heal it in smaller parts.".into());
    }
    let (w, h) = (rect.w as usize, rect.h as usize);
    let d = Block::read(dest, rect);
    let s = Block::read(source, rect.translate(dx, dy));
    let cov: Vec<f32> = coverage.read(rect);
    let mut diff = vec![[0.0f32; 4]; w * h];
    let mut known = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (px, py) = (rect.x + x as i32, rect.y + y as i32);
            let sp = s.at(px + dx, py + dy);
            let dp = d.at(px, py);
            for c in 0..4 {
                diff[i][c] = dp[c] - sp[c];
            }
            known[i] = if cov[i] > 0.0 { 0.0 } else { 1.0 };
        }
    }
    membrane(w, h, &mut diff, &known);
    let mut out = dest.clone();
    out.update(rect, |x, y, px| {
        let i = (y - rect.y) as usize * w + (x - rect.x) as usize;
        let a = cov[i].clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let sp = s.at(x + dx, y + dy);
        for c in 0..4 {
            let healed = (sp[c] + diff[i][c]).clamp(0.0, 1.0);
            px[c] += (healed - px[c]) * a;
        }
    });
    Ok(out)
}

/// how well the ring around `hole` matches the ring around it moved by (dx, dy): a sum of
/// squared differences per pixel, or none if the source runs into the hole or off `bounds`
fn ring_cost(
    img: &Raster,
    hole: IRect,
    mask: &Raster,
    dx: i32,
    dy: i32,
    bounds: IRect,
) -> Option<f32> {
    let moved = hole.translate(dx, dy);
    if !bounds.contains_rect(&moved) || !moved.intersect(&hole).is_empty() {
        return None;
    }
    let ring = hole.expand(4);
    let mut cost = 0.0;
    let mut n = 0.0;
    let step = ((ring.w.max(ring.h) / 64).max(1)) as i32;
    let mut y = ring.y;
    while y < ring.bottom() {
        let mut x = ring.x;
        while x < ring.right() {
            let on_ring = !hole.contains(x, y) || mask.get(x, y, 0) <= 0.0;
            if on_ring && bounds.contains(x, y) && bounds.contains(x + dx, y + dy) {
                for c in 0..3 {
                    let d = img.get(x, y, c) - img.get(x + dx, y + dy, c);
                    cost += d * d;
                }
                n += 1.0;
            }
            x += step;
        }
        y += step;
    }
    (n > 0.0).then(|| cost / n)
}

/// the offset content aware fill would copy from to cover `hole` (the area `mask` marks)
pub fn best_offset(img: &Raster, mask: &Raster, hole: IRect, bounds: IRect) -> Option<(i32, i32)> {
    let size = hole.w.max(hole.h) as f32;
    let mut best: Option<(f32, i32, i32)> = None;
    for ring in 1..=6 {
        let dist = size * (0.6 + 0.4 * ring as f32) + 2.0;
        let steps = 8 * ring;
        for k in 0..steps {
            let a = std::f32::consts::TAU * k as f32 / steps as f32;
            let (dx, dy) = (
                (a.cos() * dist).round() as i32,
                (a.sin() * dist).round() as i32,
            );
            if let Some(c) = ring_cost(img, hole, mask, dx, dy, bounds)
                && best.is_none_or(|b| c < b.0)
            {
                best = Some((c, dx, dy));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// fills the area `mask` marks with texture from elsewhere in `img`, healed in
pub fn content_fill(img: &Raster, mask: &Raster, bounds: IRect) -> Result<Raster, String> {
    let hole = mask.content_bounds().intersect(&bounds);
    if hole.is_empty() {
        return Ok(img.clone());
    }
    let (dx, dy) = best_offset(img, mask, hole, bounds)
        .ok_or("There is not enough picture around that area to fill it from.")?;
    heal(img, img, dx, dy, mask, hole)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Depth;

    fn stripes() -> Raster {
        let mut r = Raster::rgba(Depth::U8);
        r.update(IRect::of_size(96, 64), |x, _, px| {
            let v = if (x / 4) % 2 == 0 { 0.3 } else { 0.5 };
            px.copy_from_slice(&[v, v, v, 1.0]);
        });
        r
    }

    #[test]
    fn healing_takes_texture_and_the_destination_tone() {
        // a darker destination: the healed area should be darker texture, no seam
        let mut dest = stripes();
        dest.update(IRect::new(48, 0, 48, 64), |_, _, px| {
            for v in px.iter_mut().take(3) {
                *v *= 0.5;
            }
        });
        let mut cov = Raster::gray(Depth::U8, 0.0);
        cov.update(IRect::new(60, 20, 20, 20), |_, _, v| v[0] = 1.0);
        let out = heal(&dest, &dest, -48, 0, &cov, IRect::new(60, 20, 20, 20)).expect("heal");
        // poisson healing keeps the source's texture but moves its average to the
        // destination's: about 0.2 here, where the source averages 0.4
        let mut sum = 0.0;
        for y in 24..36 {
            for x in 64..76 {
                sum += out.get(x, y, 0);
            }
        }
        let mean = sum / 144.0;
        assert!(
            (mean - 0.2).abs() < 0.05,
            "took the destination's tone: {mean}"
        );
        let edge_jump = (out.get(60, 30, 0) - out.get(59, 30, 0)).abs();
        // the stripes themselves step by 0.1 here
        assert!(edge_jump < 0.2, "{edge_jump}");
    }

    #[test]
    fn content_fill_hides_a_blemish() {
        let mut img = stripes();
        img.update(IRect::new(40, 28, 8, 8), |_, _, px| {
            px.copy_from_slice(&[1.0, 0.0, 0.0, 1.0])
        });
        let mut mask = Raster::gray(Depth::U8, 0.0);
        mask.update(IRect::new(39, 27, 10, 10), |_, _, v| v[0] = 1.0);
        let out = content_fill(&img, &mask, IRect::of_size(96, 64)).expect("fill");
        let c = [out.get(43, 31, 0), out.get(43, 31, 1)];
        assert!((c[0] - c[1]).abs() < 0.05, "the red is gone: {c:?}");
    }

    #[test]
    fn membranes_interpolate_smoothly() {
        let (w, h) = (16, 1);
        let mut v = vec![[0.0f32; 4]; w];
        let mut k = vec![0.0f32; w];
        v[0] = [0.0; 4];
        k[0] = 1.0;
        v[15] = [1.0; 4];
        k[15] = 1.0;
        membrane(w, h, &mut v, &k);
        assert!(v.windows(2).all(|p| p[1][0] >= p[0][0] - 1e-4));
        assert!((v[8][0] - 0.5).abs() < 0.2);
    }
}
