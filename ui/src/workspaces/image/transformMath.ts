// the maths behind free transform, crop, snapping and the rulers. it mirrors
// crates/vellum-image/src/transform.rs where the two must agree (the corner map and the
// warp patch), so what the overlay draws lands where rust puts the pixels.

export type Pt = [number, number];
export type Quad = [Pt, Pt, Pt, Pt];
/** a 3x3 matrix, row by row */
export type M3 = [number, number, number, number, number, number, number, number, number];

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export function rectQuad(b: Box): Quad {
  return [
    [b.x, b.y],
    [b.x + b.w, b.y],
    [b.x + b.w, b.y + b.h],
    [b.x, b.y + b.h],
  ];
}

function det(m: M3): number {
  return (
    m[0] * (m[4] * m[8] - m[5] * m[7]) -
    m[1] * (m[3] * m[8] - m[5] * m[6]) +
    m[2] * (m[3] * m[7] - m[4] * m[6])
  );
}

export function invert(m: M3): M3 | null {
  const d = det(m);
  if (Math.abs(d) < 1e-12) return null;
  return [
    (m[4] * m[8] - m[5] * m[7]) / d,
    -(m[1] * m[8] - m[2] * m[7]) / d,
    (m[1] * m[5] - m[2] * m[4]) / d,
    -(m[3] * m[8] - m[5] * m[6]) / d,
    (m[0] * m[8] - m[2] * m[6]) / d,
    -(m[0] * m[5] - m[2] * m[3]) / d,
    (m[3] * m[7] - m[4] * m[6]) / d,
    -(m[0] * m[7] - m[1] * m[6]) / d,
    (m[0] * m[4] - m[1] * m[3]) / d,
  ];
}

export function multiply(a: M3, b: M3): M3 {
  const out: number[] = [];
  for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      let v = 0;
      for (let k = 0; k < 3; k++) v += (a[i * 3 + k] ?? 0) * (b[k * 3 + j] ?? 0);
      out.push(v);
    }
  }
  return out as M3;
}

export function apply(m: M3, p: Pt): Pt | null {
  const w = m[6] * p[0] + m[7] * p[1] + m[8];
  if (w <= 1e-9) return null;
  return [(m[0] * p[0] + m[1] * p[1] + m[2]) / w, (m[3] * p[0] + m[4] * p[1] + m[5]) / w];
}

/** the projective map from the unit square onto a quad, or null when it folds over */
export function squareToQuad(q: Quad): M3 | null {
  const [p0, p1, p2, p3] = q;
  const sx = p0[0] - p1[0] + p2[0] - p3[0];
  const sy = p0[1] - p1[1] + p2[1] - p3[1];
  let m: M3;
  if (Math.abs(sx) < 1e-12 && Math.abs(sy) < 1e-12) {
    m = [p1[0] - p0[0], p3[0] - p0[0], p0[0], p1[1] - p0[1], p3[1] - p0[1], p0[1], 0, 0, 1];
  } else {
    const dx1 = p1[0] - p2[0];
    const dx2 = p3[0] - p2[0];
    const dy1 = p1[1] - p2[1];
    const dy2 = p3[1] - p2[1];
    const d = dx1 * dy2 - dx2 * dy1;
    if (Math.abs(d) < 1e-12) return null;
    const g = (sx * dy2 - dx2 * sy) / d;
    const h = (dx1 * sy - sx * dy1) / d;
    m = [
      p1[0] - p0[0] + g * p1[0],
      p3[0] - p0[0] + h * p3[0],
      p0[0],
      p1[1] - p0[1] + g * p1[1],
      p3[1] - p0[1] + h * p3[1],
      p0[1],
      g,
      h,
      1,
    ];
  }
  for (const [u, v] of [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ] as const) {
    if (m[6] * u + m[7] * v + m[8] <= 1e-9) return null;
  }
  if (Math.abs(det(m)) < 1e-12) return null;
  return m;
}

export function quadToQuad(from: Quad, to: Quad): M3 | null {
  const a = squareToQuad(from);
  const b = squareToQuad(to);
  const ai = a && invert(a);
  return ai && b ? multiply(b, ai) : null;
}

/** true when no corner folds back over the others */
export function isConvex(q: Quad): boolean {
  let sign = 0;
  for (let i = 0; i < 4; i++) {
    const a = q[i] as Pt;
    const b = q[(i + 1) % 4] as Pt;
    const c = q[(i + 2) % 4] as Pt;
    const cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
    if (Math.abs(cross) < 1e-9) return false;
    const s = Math.sign(cross);
    if (sign === 0) sign = s;
    else if (s !== sign) return false;
  }
  return true;
}

/** a point of the warp patch at (u, v) in [0, 1], like `warp_at` in rust */
export function warpAt(points: readonly Pt[], u: number, v: number): Pt {
  const b = (t: number) => {
    const s = 1 - t;
    return [s * s * s, 3 * s * s * t, 3 * s * t * t, t * t * t];
  };
  const bu = b(u);
  const bv = b(v);
  let x = 0;
  let y = 0;
  for (let j = 0; j < 4; j++) {
    for (let i = 0; i < 4; i++) {
      const p = points[j * 4 + i];
      const w = (bu[i] ?? 0) * (bv[j] ?? 0);
      if (p) {
        x += p[0] * w;
        y += p[1] * w;
      }
    }
  }
  return [x, y];
}

/** the warp points that give the same picture as the corners, to start a warp from */
export function warpFromQuad(source: Box, corners: Quad): Pt[] {
  const m = quadToQuad(rectQuad(source), corners);
  const out: Pt[] = [];
  for (let j = 0; j < 4; j++) {
    for (let i = 0; i < 4; i++) {
      const p: Pt = [source.x + (source.w * i) / 3, source.y + (source.h * j) / 3];
      out.push((m && apply(m, p)) ?? p);
    }
  }
  return out;
}

/** where a document point inside `source` lands */
export function mapPoint(source: Box, corners: Quad, warp: readonly Pt[] | null, p: Pt): Pt | null {
  if (warp && warp.length === 16) {
    return warpAt(warp, (p[0] - source.x) / source.w, (p[1] - source.y) / source.h);
  }
  const m = quadToQuad(rectQuad(source), corners);
  return m ? apply(m, p) : null;
}

export const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
export const sub = (a: Pt, b: Pt): Pt => [a[0] - b[0], a[1] - b[1]];
export const scale = (a: Pt, s: number): Pt => [a[0] * s, a[1] * s];
export const dot = (a: Pt, b: Pt) => a[0] * b[0] + a[1] * b[1];
export const len = (a: Pt) => Math.hypot(a[0], a[1]);

export function rotateAbout(p: Pt, c: Pt, deg: number): Pt {
  const r = (deg * Math.PI) / 180;
  const s = Math.sin(r);
  const k = Math.cos(r);
  const d = sub(p, c);
  return [c[0] + d[0] * k - d[1] * s, c[1] + d[0] * s + d[1] * k];
}

export function centre(q: readonly Pt[]): Pt {
  const n = q.length || 1;
  return [q.reduce((s, p) => s + p[0], 0) / n, q.reduce((s, p) => s + p[1], 0) / n];
}

/** what the numeric fields show: an affine box as centre, size, angle and skew */
export interface Affine {
  x: number;
  y: number;
  /** percent of the source box */
  w: number;
  h: number;
  /** degrees, clockwise */
  angle: number;
  /** degrees the left edge leans over */
  skew: number;
}

/** true when the corners still make a parallelogram, so the numbers describe them */
export function isParallelogram(q: Quad): boolean {
  const a = add(q[0], q[2]);
  const b = add(q[1], q[3]);
  return len(sub(a, b)) < 1e-3 * Math.max(1, len(sub(q[2], q[0])));
}

export function toAffine(source: Box, q: Quad): Affine {
  const e1 = sub(q[1], q[0]);
  const e2 = sub(q[3], q[0]);
  const angle = (Math.atan2(e1[1], e1[0]) * 180) / Math.PI;
  const r = (-angle * Math.PI) / 180;
  const local: Pt = [
    e2[0] * Math.cos(r) - e2[1] * Math.sin(r),
    e2[0] * Math.sin(r) + e2[1] * Math.cos(r),
  ];
  const c = centre(q);
  return {
    x: c[0],
    y: c[1],
    w: (len(e1) / source.w) * 100,
    h: (local[1] / source.h) * 100,
    angle,
    skew: (Math.atan2(local[0], local[1]) * 180) / Math.PI,
  };
}

export function fromAffine(source: Box, a: Affine): Quad {
  const sx = a.w / 100;
  const sy = a.h / 100;
  const k = Math.tan((a.skew * Math.PI) / 180);
  const hw = source.w / 2;
  const hh = source.h / 2;
  const corners: Pt[] = [
    [-hw, -hh],
    [hw, -hh],
    [hw, hh],
    [-hw, hh],
  ].map(([x = 0, y = 0]) => {
    const lx = x * sx + y * sy * k;
    const ly = y * sy;
    return rotateAbout([a.x + lx, a.y + ly], [a.x, a.y], a.angle);
  });
  return corners as Quad;
}

/** what dragging a handle does */
export type HandleMode = "scale" | "rotate" | "skew" | "distort" | "perspective";

/**
 * moves corner `k` to `p`. scale keeps the opposite corner where it is (or the centre
 * with `fromCentre`) and the box's shape; `proportional` keeps its aspect too. distort
 * moves the corner alone; perspective moves the corner and mirrors the move on its
 * neighbour across the same edge.
 */
export function dragCorner(
  start: Quad,
  k: number,
  p: Pt,
  mode: HandleMode,
  proportional: boolean,
  fromCentre: boolean,
): Quad {
  const q = start.map((c) => [c[0], c[1]] as Pt) as Quad;
  if (mode === "distort") {
    q[k] = p;
    return q;
  }
  if (mode === "perspective") {
    // the neighbour on the same horizontal edge moves the other way along that edge
    const n = k ^ 1;
    const edge = sub(corner(start, n), corner(start, k));
    const unit = scale(edge, 1 / (len(edge) || 1));
    const along = dot(sub(p, corner(start, k)), unit);
    q[k] = add(corner(start, k), scale(unit, along));
    q[n] = sub(corner(start, n), scale(unit, along));
    return q;
  }
  const opp = (k + 2) % 4;
  const anchor = fromCentre ? centre(start) : corner(start, opp);
  const u = sub(corner(start, 1), corner(start, 0));
  const v = sub(corner(start, 3), corner(start, 0));
  const lu = len(u) || 1;
  const lv = len(v) || 1;
  const uu = scale(u, 1 / lu);
  const vv = scale(v, 1 / lv);
  const from = sub(corner(start, k), anchor);
  const to = sub(p, anchor);
  let fu = dot(from, uu) === 0 ? 1 : dot(to, uu) / dot(from, uu);
  let fv = dot(from, vv) === 0 ? 1 : dot(to, vv) / dot(from, vv);
  if (proportional) {
    const f = Math.abs(fu) > Math.abs(fv) ? fu : fv;
    fu = Math.sign(fu || 1) * Math.abs(f);
    fv = Math.sign(fv || 1) * Math.abs(f);
  }
  return scaleQuad(start, anchor, uu, vv, fu, fv);
}

function scaleQuad(start: Quad, anchor: Pt, uu: Pt, vv: Pt, fu: number, fv: number): Quad {
  return start.map((c) => {
    const d = sub(c, anchor);
    const a = dot(d, uu);
    const b = dot(d, vv);
    // the two axes may lean (a skewed box), so rebuild through them, not x and y
    const cross = uu[0] * vv[1] - uu[1] * vv[0];
    const ca = Math.abs(cross) < 1e-9 ? a : (d[0] * vv[1] - d[1] * vv[0]) / cross;
    const cb = Math.abs(cross) < 1e-9 ? b : (uu[0] * d[1] - uu[1] * d[0]) / cross;
    return add(anchor, add(scale(uu, ca * fu), scale(vv, cb * fv)));
  }) as Quad;
}

/**
 * moves edge `e` (0 top, 1 right, 2 bottom, 3 left) towards `p`: scale moves it
 * across, skew slides it along itself
 */
export function dragEdge(
  start: Quad,
  e: number,
  p: Pt,
  mode: HandleMode,
  fromCentre: boolean,
): Quad {
  const a = e;
  const b = (e + 1) % 4;
  const mid = scale(add(corner(start, a), corner(start, b)), 0.5);
  const q = start.map((c) => [c[0], c[1]] as Pt) as Quad;
  if (mode === "skew" || mode === "distort" || mode === "perspective") {
    const along = sub(corner(start, b), corner(start, a));
    const unit = scale(along, 1 / (len(along) || 1));
    const shift = scale(unit, dot(sub(p, mid), unit));
    q[a] = add(corner(start, a), shift);
    q[b] = add(corner(start, b), shift);
    return q;
  }
  const oppMid = scale(add(corner(start, (e + 2) % 4), corner(start, (e + 3) % 4)), 0.5);
  const anchor = fromCentre ? centre(start) : oppMid;
  const u = sub(corner(start, 1), corner(start, 0));
  const v = sub(corner(start, 3), corner(start, 0));
  const uu = scale(u, 1 / (len(u) || 1));
  const vv = scale(v, 1 / (len(v) || 1));
  const axis = e % 2 === 0 ? vv : uu;
  const from = dot(sub(mid, anchor), axis);
  const to = dot(sub(p, anchor), axis);
  const f = from === 0 ? 1 : to / from;
  return e % 2 === 0
    ? scaleQuad(start, anchor, uu, vv, 1, f)
    : scaleQuad(start, anchor, uu, vv, f, 1);
}

/** corner `i` of a quad, counting round from the top left */
export function corner(q: Quad, i: number): Pt {
  return q[((i % 4) + 4) % 4] as Pt;
}

export function moveQuad(q: Quad, d: Pt): Quad {
  return q.map((c) => add(c, d)) as Quad;
}

export function rotateQuad(q: Quad, pivot: Pt, deg: number): Quad {
  return q.map((c) => rotateAbout(c, pivot, deg)) as Quad;
}

/** flips a quad in place across its own middle */
export function flipQuad(q: Quad, horizontal: boolean): Quad {
  return horizontal ? [q[1], q[0], q[3], q[2]] : [q[3], q[2], q[1], q[0]];
}

/** the size a perspective crop gives: the average of its opposite sides */
export function perspectiveSize(q: Quad): [number, number] {
  const w = (len(sub(q[1], q[0])) + len(sub(q[2], q[3]))) / 2;
  const h = (len(sub(q[3], q[0])) + len(sub(q[2], q[1]))) / 2;
  return [Math.max(1, Math.round(w)), Math.max(1, Math.round(h))];
}

/** a crop box turned about its middle: centre, size and angle */
export interface CropBox {
  cx: number;
  cy: number;
  w: number;
  h: number;
  angle: number;
}

export function cropQuad(c: CropBox): Quad {
  const hw = c.w / 2;
  const hh = c.h / 2;
  return [
    [-hw, -hh],
    [hw, -hh],
    [hw, hh],
    [-hw, hh],
  ].map(([x = 0, y = 0]) => rotateAbout([c.cx + x, c.cy + y], [c.cx, c.cy], c.angle)) as Quad;
}

/**
 * the biggest box with this one's shape, centre and angle that stays inside a canvas of
 * `cw` by `ch`, so straightening never shows the see through corners
 */
export function fitInside(c: CropBox, cw: number, ch: number): CropBox {
  const r = (c.angle * Math.PI) / 180;
  const s = Math.abs(Math.sin(r));
  const k = Math.abs(Math.cos(r));
  // the room from the centre to each canvas side
  const rx = Math.min(c.cx, cw - c.cx);
  const ry = Math.min(c.cy, ch - c.cy);
  const needX = (c.w / 2) * k + (c.h / 2) * s;
  const needY = (c.w / 2) * s + (c.h / 2) * k;
  const f = Math.min(1, needX > 0 ? rx / needX : 1, needY > 0 ? ry / needY : 1);
  return { ...c, w: c.w * f, h: c.h * f };
}

/** ratios the crop tool offers, width over height; 0 means free */
export const ASPECTS: [string, number][] = [
  ["Free", 0],
  ["Original", -1],
  ["1:1", 1],
  ["4:5", 4 / 5],
  ["5:4", 5 / 4],
  ["3:2", 3 / 2],
  ["2:3", 2 / 3],
  ["4:3", 4 / 3],
  ["16:9", 16 / 9],
  ["9:16", 9 / 16],
];

/** lines a moving thing can snap to, in document pixels */
export interface SnapTargets {
  xs: number[];
  ys: number[];
}

/** what snapping did: the corrected move and the lines it lined up with */
export interface Snap {
  dx: number;
  dy: number;
  lines: { vertical: boolean; at: number }[];
}

/**
 * nudges a move so one of the moving `edges` lands on a target when it comes within
 * `tolerance` document pixels. x and y snap on their own.
 */
export function snapMove(
  edges: SnapTargets,
  targets: SnapTargets,
  dx: number,
  dy: number,
  tolerance: number,
): Snap {
  const best = (moving: number[], fixed: number[], d: number) => {
    let found: { off: number; at: number } | null = null;
    for (const m of moving) {
      for (const t of fixed) {
        const off = t - (m + d);
        if (Math.abs(off) <= tolerance && (!found || Math.abs(off) < Math.abs(found.off))) {
          found = { off, at: t };
        }
      }
    }
    return found;
  };
  const bx = best(edges.xs, targets.xs, dx);
  const by = best(edges.ys, targets.ys, dy);
  const lines: Snap["lines"] = [];
  if (bx) lines.push({ vertical: true, at: bx.at });
  if (by) lines.push({ vertical: false, at: by.at });
  return { dx: dx + (bx?.off ?? 0), dy: dy + (by?.off ?? 0), lines };
}

/** a box's left, middle and right, and top, middle and bottom */
export function boxEdges(b: Box): SnapTargets {
  return { xs: [b.x, b.x + b.w / 2, b.x + b.w], ys: [b.y, b.y + b.h / 2, b.y + b.h] };
}

/** a nice spacing for ruler ticks so labels sit at least `minPx` apart on screen */
export function rulerStep(zoom: number, minPx: number): number {
  const steps = [1, 2, 5];
  for (let p = 1; p < 1e7; p *= 10) {
    for (const s of steps) {
      if (s * p * zoom >= minPx) return s * p;
    }
  }
  return 1e7;
}
