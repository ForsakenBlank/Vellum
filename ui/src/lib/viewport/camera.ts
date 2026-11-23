// where the viewport looks. the maths mirrors `ViewState` in crates/vellum-gpu/src/view.rs,
// so the tiles rust picks always cover what the window draws. sizes are device pixels.

export interface Camera {
  /** the document pixel at the centre of the view */
  centerX: number;
  centerY: number;
  /** device pixels per document pixel */
  zoom: number;
  /** clockwise, in degrees */
  rotation: number;
  flipX: boolean;
}

export interface Size {
  width: number;
  height: number;
}

export const MIN_ZOOM = 1 / 256;
export const MAX_ZOOM = 256;

/** the zoom steps the buttons and keys move through, like photoshop's */
export const ZOOM_STEPS = [
  1 / 64,
  1 / 48,
  1 / 32,
  1 / 24,
  1 / 16,
  1 / 12,
  1 / 8,
  1 / 6,
  1 / 4,
  1 / 3,
  1 / 2,
  2 / 3,
  1,
  2,
  3,
  4,
  5,
  6,
  7,
  8,
  12,
  16,
  20,
  24,
  32,
  64,
  128,
  256,
];

export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));
}

/** keeps an angle in (-180, 180] */
export function normalizeAngle(deg: number): number {
  let a = deg % 360;
  if (a > 180) a -= 360;
  if (a <= -180) a += 360;
  return a;
}

export function toScreen(cam: Camera, size: Size, x: number, y: number): [number, number] {
  const r = (cam.rotation * Math.PI) / 180;
  const s = Math.sin(r);
  const c = Math.cos(r);
  const dx = (x - cam.centerX) * cam.zoom * (cam.flipX ? -1 : 1);
  const dy = (y - cam.centerY) * cam.zoom;
  return [dx * c - dy * s + size.width / 2, dx * s + dy * c + size.height / 2];
}

export function toDocument(cam: Camera, size: Size, sx: number, sy: number): [number, number] {
  const r = (cam.rotation * Math.PI) / 180;
  const s = Math.sin(r);
  const c = Math.cos(r);
  const px = sx - size.width / 2;
  const py = sy - size.height / 2;
  const dx = px * c + py * s;
  const dy = -px * s + py * c;
  return [cam.centerX + (dx / cam.zoom) * (cam.flipX ? -1 : 1), cam.centerY + dy / cam.zoom];
}

/** zooms by `factor`, keeping the document point under (sx, sy) where it is */
export function zoomAt(cam: Camera, size: Size, sx: number, sy: number, factor: number): Camera {
  const zoom = clampZoom(cam.zoom * factor);
  const [x, y] = toDocument(cam, size, sx, sy);
  const next = { ...cam, zoom };
  const [nx, ny] = toDocument(next, size, sx, sy);
  return { ...next, centerX: cam.centerX + x - nx, centerY: cam.centerY + y - ny };
}

/** zooms to an exact level, keeping (sx, sy) in place */
export function zoomTo(cam: Camera, size: Size, sx: number, sy: number, zoom: number): Camera {
  return zoomAt(cam, size, sx, sy, clampZoom(zoom) / cam.zoom);
}

/** the next zoom step in or out from `zoom` */
export function stepZoom(zoom: number, direction: 1 | -1): number {
  const eps = 1e-6;
  if (direction > 0) return ZOOM_STEPS.find((z) => z > zoom * (1 + eps)) ?? MAX_ZOOM;
  return [...ZOOM_STEPS].reverse().find((z) => z < zoom * (1 - eps)) ?? MIN_ZOOM;
}

/** moves the view so the picture follows a drag of (dx, dy) device pixels */
export function panBy(cam: Camera, dx: number, dy: number): Camera {
  const r = (cam.rotation * Math.PI) / 180;
  const s = Math.sin(r);
  const c = Math.cos(r);
  const ux = (dx * c + dy * s) / cam.zoom;
  const uy = (-dx * s + dy * c) / cam.zoom;
  return { ...cam, centerX: cam.centerX - ux * (cam.flipX ? -1 : 1), centerY: cam.centerY - uy };
}

/** turns the view by `deg`, keeping the document point under (sx, sy) where it is */
export function rotateAround(cam: Camera, size: Size, sx: number, sy: number, deg: number): Camera {
  const [x, y] = toDocument(cam, size, sx, sy);
  const next = { ...cam, rotation: normalizeAngle(cam.rotation + deg) };
  const [nx, ny] = toDocument(next, size, sx, sy);
  return { ...next, centerX: cam.centerX + x - nx, centerY: cam.centerY + y - ny };
}

/** mirrors the view left to right about the centre of the window */
export function flip(cam: Camera): Camera {
  return { ...cam, flipX: !cam.flipX };
}

/** the whole document in view with a margin, upright, never past 100 percent */
export function fit(doc: Size, size: Size, margin = 24): Camera {
  const w = Math.max(1, size.width - margin * 2);
  const h = Math.max(1, size.height - margin * 2);
  const zoom = clampZoom(Math.min(w / Math.max(1, doc.width), h / Math.max(1, doc.height), 1));
  return { centerX: doc.width / 2, centerY: doc.height / 2, zoom, rotation: 0, flipX: false };
}

/** the mip level rust renders for a zoom, see `level_for_zoom` in view.rs */
export function levelForZoom(zoom: number): number {
  if (!Number.isFinite(zoom) || zoom >= 1 || zoom <= 0) return 0;
  return Math.min(24, Math.floor(Math.log2(1 / zoom)));
}

/**
 * the 3x3 matrix (column major, for webgl) that takes document pixels to clip space.
 * y points down in documents and up in clip space.
 */
export function clipMatrix(cam: Camera, size: Size): Float32Array {
  const r = (cam.rotation * Math.PI) / 180;
  const s = Math.sin(r);
  const c = Math.cos(r);
  const fx = cam.flipX ? -1 : 1;
  // screen = a * doc + b, where a = rot * diag(zoom * fx, zoom)
  const a00 = c * cam.zoom * fx;
  const a01 = -s * cam.zoom;
  const a10 = s * cam.zoom * fx;
  const a11 = c * cam.zoom;
  const bx = size.width / 2 - (a00 * cam.centerX + a01 * cam.centerY);
  const by = size.height / 2 - (a10 * cam.centerX + a11 * cam.centerY);
  // clip = (screen.x / w * 2 - 1, 1 - screen.y / h * 2)
  const kx = 2 / size.width;
  const ky = -2 / size.height;
  return new Float32Array([
    a00 * kx,
    a10 * ky,
    0,
    a01 * kx,
    a11 * ky,
    0,
    bx * kx - 1,
    by * ky + 1,
    1,
  ]);
}

/** a zoom as people say it: "100%", "33.3%", "1600%" */
export function zoomLabel(zoom: number): string {
  const pct = zoom * 100;
  if (pct >= 100) return `${String(Math.round(pct))}%`;
  if (pct >= 10) return `${pct.toFixed(1).replace(/\.0$/, "")}%`;
  return `${pct.toFixed(2).replace(/0$/, "").replace(/\.0$/, "")}%`;
}
