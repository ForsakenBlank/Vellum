import type { Combine } from "../../bindings/Combine";
import type { LayerState } from "../../bindings/LayerState";

// pure helpers for the image editor, kept apart from react so they are easy to test

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * the marquee box for a drag, photoshop style: shift makes it square, alt draws it from
 * the centre out
 */
export function marqueeBox(
  start: [number, number],
  end: [number, number],
  square: boolean,
  fromCentre: boolean,
): Box {
  let dx = end[0] - start[0];
  let dy = end[1] - start[1];
  if (square) {
    const side = Math.max(Math.abs(dx), Math.abs(dy));
    dx = Math.sign(dx || 1) * side;
    dy = Math.sign(dy || 1) * side;
  }
  if (fromCentre) {
    return {
      x: start[0] - Math.abs(dx),
      y: start[1] - Math.abs(dy),
      w: Math.abs(dx) * 2,
      h: Math.abs(dy) * 2,
    };
  }
  return {
    x: Math.min(start[0], start[0] + dx),
    y: Math.min(start[1], start[1] + dy),
    w: Math.abs(dx),
    h: Math.abs(dy),
  };
}

/** whole pixels, since marquees snap to the pixel grid like photoshop's */
export function snapBox(b: Box): Box {
  const x0 = Math.round(b.x);
  const y0 = Math.round(b.y);
  return { x: x0, y: y0, w: Math.round(b.x + b.w) - x0, h: Math.round(b.y + b.h) - y0 };
}

/** holding shift adds to the selection, alt takes away, both keep only the overlap */
export function combineFor(base: Combine, shift: boolean, alt: boolean): Combine {
  if (shift && alt) return "intersect";
  if (shift) return "add";
  if (alt) return "subtract";
  return base;
}

/** the brush sizes [ and ] step through, like photoshop's */
export function stepSize(size: number, direction: 1 | -1): number {
  const step = size < 10 ? 1 : size < 100 ? 10 : size < 200 ? 25 : size < 300 ? 50 : 100;
  const next =
    direction > 0
      ? size + step
      : size - (size <= 10 ? 1 : size <= 100 ? 10 : size <= 200 ? 25 : size <= 300 ? 50 : 100);
  return Math.min(5000, Math.max(1, Math.round(next)));
}

export function stepHardness(hardness: number, direction: 1 | -1): number {
  return Math.min(1, Math.max(0, Math.round((hardness + direction * 0.25) * 4) / 4));
}

/** with shift held, a line snaps to the nearest 45 degrees */
export function snapAngle(start: [number, number], end: [number, number]): [number, number] {
  const dx = end[0] - start[0];
  const dy = end[1] - start[1];
  const len = Math.hypot(dx, dy);
  const step = Math.PI / 4;
  const a = Math.round(Math.atan2(dy, dx) / step) * step;
  return [start[0] + Math.cos(a) * len, start[1] + Math.sin(a) * len];
}

export function cycle<T>(list: readonly T[], current: T): T {
  const i = list.indexOf(current);
  return list[(i + 1) % list.length] ?? current;
}

export function toHex(c: readonly number[]): string {
  return (
    "#" +
    c
      .slice(0, 3)
      .map((v) =>
        Math.round(Math.min(1, Math.max(0, v)) * 255)
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
  );
}

export function fromHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6}|[0-9a-f]{3})$/i.exec(hex.trim());
  if (!m?.[1]) return null;
  const h = m[1].length === 3 ? m[1].replace(/./g, (c) => c + c) : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as [number, number, number];
}

export function hsvToRgb(h: number, s: number, v: number): [number, number, number] {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  switch (((i % 6) + 6) % 6) {
    case 0:
      return [v, t, p];
    case 1:
      return [q, v, p];
    case 2:
      return [p, v, t];
    case 3:
      return [p, q, v];
    case 4:
      return [t, p, v];
    default:
      return [v, p, q];
  }
}

export function rgbToHsv(c: readonly number[]): [number, number, number] {
  const [r = 0, g = 0, b = 0] = c;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d > 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
    if (h < 0) h += 1;
  }
  return [h, max === 0 ? 0 : d / max, max];
}

/** one row of the layers panel */
export interface LayerRow {
  layer: LayerState;
  depth: number;
  /** the group this layer is in, null at the top */
  parent: number | null;
  /** its index in that group's list, counted from the bottom */
  index: number;
}

/**
 * the layers panel's rows, top of the stack first, children under their group. a search
 * shows every match whatever groups are collapsed, and matches names ignoring case;
 * "group", "pixels", "masked", "hidden", "clipped", "text" and colour names match what
 * they say, the same as `search_layers` in rust.
 */
export function layerRows(layers: readonly LayerState[], query = ""): LayerRow[] {
  const q = query.trim().toLowerCase();
  const rows: LayerRow[] = [];
  const matches = (l: LayerState) =>
    !q ||
    l.name.toLowerCase().includes(q) ||
    (q === "group" && l.group) ||
    (q === "pixels" && !l.group) ||
    (q === "masked" && l.mask !== null) ||
    (q === "hidden" && !l.visible) ||
    (q === "clipped" && l.clipped) ||
    l.origin === q ||
    (l.label !== "none" && l.label === q);
  const walk = (list: readonly LayerState[], depth: number, parent: number | null) => {
    for (let i = list.length - 1; i >= 0; i--) {
      const l = list[i];
      if (!l) continue;
      if (matches(l)) rows.push({ layer: l, depth: q ? 0 : depth, parent, index: i });
      if (l.group && (q || l.expanded)) walk(l.children, depth + 1, l.id);
    }
  };
  walk(layers, 0, null);
  return rows;
}

/** finds a layer anywhere in the tree */
export function findLayer(layers: readonly LayerState[], id: number): LayerState | undefined {
  for (const l of layers) {
    if (l.id === id) return l;
    const inside = findLayer(l.children, id);
    if (inside) return inside;
  }
  return undefined;
}

export interface Thumb {
  width: number;
  height: number;
  pixels: Uint8ClampedArray<ArrayBuffer>;
}

/** reads `image_thumbnails`: (u32 layer, u8 mask, u32 w, u32 h, rgba) repeated */
export function parseThumbnails(buffer: ArrayBuffer): Map<string, Thumb> {
  const out = new Map<string, Thumb>();
  const view = new DataView(buffer);
  let at = 0;
  while (at + 13 <= buffer.byteLength) {
    const layer = view.getUint32(at, true);
    const mask = view.getUint8(at + 4) === 1;
    const width = view.getUint32(at + 5, true);
    const height = view.getUint32(at + 9, true);
    at += 13;
    const len = width * height * 4;
    if (at + len > buffer.byteLength) break;
    out.set(thumbKey(layer, mask), {
      width,
      height,
      pixels: new Uint8ClampedArray(buffer.slice(at, at + len)),
    });
    at += len;
  }
  return out;
}

export function thumbKey(layer: number, mask: boolean): string {
  return `${String(layer)}${mask ? "m" : ""}`;
}
