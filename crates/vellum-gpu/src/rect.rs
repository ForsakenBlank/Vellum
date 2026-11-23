//! integer rectangles in pixel space.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// a rectangle of whole pixels. `w` and `h` of zero is the empty rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl IRect {
    pub const EMPTY: IRect = IRect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// from two corners, `x1` and `y1` exclusive
    pub fn from_corners(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        if x1 <= x0 || y1 <= y0 {
            return Self::EMPTY;
        }
        Self::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    pub fn of_size(w: u32, h: u32) -> Self {
        Self::new(0, 0, w, h)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn area(&self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn contains_rect(&self, other: &IRect) -> bool {
        other.is_empty()
            || (other.x >= self.x
                && other.y >= self.y
                && other.right() <= self.right()
                && other.bottom() <= self.bottom())
    }

    pub fn intersect(&self, other: &IRect) -> IRect {
        Self::from_corners(
            self.x.max(other.x),
            self.y.max(other.y),
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        )
    }

    pub fn intersects(&self, other: &IRect) -> bool {
        !self.intersect(other).is_empty()
    }

    /// the smallest rectangle holding both. an empty side is ignored.
    pub fn union(&self, other: &IRect) -> IRect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Self::from_corners(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// grows every side by `by` pixels (shrinks for negative values)
    pub fn expand(&self, by: i32) -> IRect {
        if self.is_empty() {
            return *self;
        }
        Self::from_corners(
            self.x - by,
            self.y - by,
            self.right() + by,
            self.bottom() + by,
        )
    }

    pub fn translate(&self, dx: i32, dy: i32) -> IRect {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// the rectangle at a smaller mip level: level 1 is half size. rounds outwards so the
    /// result covers every source pixel.
    pub fn at_level(&self, level: u32) -> IRect {
        if level == 0 || self.is_empty() {
            return *self;
        }
        let d = 1i64 << level;
        let x0 = (i64::from(self.x)).div_euclid(d);
        let y0 = (i64::from(self.y)).div_euclid(d);
        let x1 = (i64::from(self.right()) + d - 1).div_euclid(d);
        let y1 = (i64::from(self.bottom()) + d - 1).div_euclid(d);
        Self::from_corners(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
    }

    /// the level 0 rectangle covered by this rectangle at `level`
    pub fn to_level0(&self, level: u32) -> IRect {
        if level == 0 || self.is_empty() {
            return *self;
        }
        let d = 1i32 << level.min(30);
        Self::new(
            self.x.saturating_mul(d),
            self.y.saturating_mul(d),
            self.w.saturating_mul(d as u32),
            self.h.saturating_mul(d as u32),
        )
    }

    /// splits the rectangle into tiles of at most `size` pixels on a side, aligned to a
    /// grid that starts at the origin, so neighbouring requests share tile edges.
    pub fn tiles(&self, size: u32) -> Vec<IRect> {
        let mut out = Vec::new();
        if self.is_empty() || size == 0 {
            return out;
        }
        let s = size as i32;
        let tx0 = self.x.div_euclid(s);
        let ty0 = self.y.div_euclid(s);
        let tx1 = (self.right() - 1).div_euclid(s);
        let ty1 = (self.bottom() - 1).div_euclid(s);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let tile = IRect::new(tx * s, ty * s, size, size).intersect(self);
                if !tile.is_empty() {
                    out.push(tile);
                }
            }
        }
        out
    }
}

/// bounds of what a node produces. most nodes are finite; a solid colour fills everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bounds {
    Empty,
    Rect(IRect),
    Infinite,
}

impl Bounds {
    pub fn of(rect: IRect) -> Self {
        if rect.is_empty() {
            Self::Empty
        } else {
            Self::Rect(rect)
        }
    }

    pub fn clip(&self, region: IRect) -> IRect {
        match self {
            Bounds::Empty => IRect::EMPTY,
            Bounds::Rect(r) => r.intersect(&region),
            Bounds::Infinite => region,
        }
    }

    pub fn union(self, other: Bounds) -> Bounds {
        match (self, other) {
            (Bounds::Infinite, _) | (_, Bounds::Infinite) => Bounds::Infinite,
            (Bounds::Empty, b) | (b, Bounds::Empty) => b,
            (Bounds::Rect(a), Bounds::Rect(b)) => Bounds::Rect(a.union(&b)),
        }
    }

    pub fn intersect(self, other: Bounds) -> Bounds {
        match (self, other) {
            (Bounds::Empty, _) | (_, Bounds::Empty) => Bounds::Empty,
            (Bounds::Infinite, b) | (b, Bounds::Infinite) => b,
            (Bounds::Rect(a), Bounds::Rect(b)) => Bounds::of(a.intersect(&b)),
        }
    }

    pub fn expand(self, by: i32) -> Bounds {
        match self {
            Bounds::Rect(r) => Bounds::of(r.expand(by)),
            other => other,
        }
    }

    pub fn at_level(self, level: u32) -> Bounds {
        match self {
            Bounds::Rect(r) => Bounds::of(r.at_level(level)),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_and_union() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), IRect::new(5, 5, 5, 5));
        assert_eq!(a.union(&b), IRect::new(0, 0, 15, 15));
        assert!(a.intersect(&IRect::new(20, 0, 5, 5)).is_empty());
        assert_eq!(IRect::EMPTY.union(&a), a);
    }

    #[test]
    fn levels_round_outwards() {
        let r = IRect::new(-3, 1, 7, 6);
        assert_eq!(r.at_level(1), IRect::from_corners(-2, 0, 2, 4));
        assert_eq!(IRect::new(0, 0, 30000, 30000).at_level(3).w, 3750);
        assert_eq!(IRect::new(1, 1, 2, 2).to_level0(2), IRect::new(4, 4, 8, 8));
    }

    #[test]
    fn tiles_cover_the_rect_exactly() {
        let r = IRect::new(-5, 3, 600, 300);
        let tiles = r.tiles(256);
        let area: u64 = tiles.iter().map(IRect::area).sum();
        assert_eq!(area, r.area());
        assert!(tiles.iter().all(|t| r.contains_rect(t)));
        assert_eq!(tiles[0], IRect::new(-5, 3, 5, 253));
    }

    #[test]
    fn bounds_algebra() {
        let a = Bounds::of(IRect::new(0, 0, 4, 4));
        assert_eq!(a.union(Bounds::Infinite), Bounds::Infinite);
        assert_eq!(a.intersect(Bounds::Infinite), a);
        assert_eq!(Bounds::Empty.union(a), a);
        assert_eq!(
            Bounds::Infinite.clip(IRect::new(1, 1, 2, 2)),
            IRect::new(1, 1, 2, 2)
        );
    }
}
