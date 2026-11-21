import { describe, expect, it } from "vitest";

import {
  type Camera,
  clipMatrix,
  fit,
  flip,
  levelForZoom,
  panBy,
  rotateAround,
  stepZoom,
  toDocument,
  toScreen,
  zoomAt,
  zoomLabel,
} from "./camera";
import { packFrame, parseFrame, tileKey } from "./frame";
import { TileCache } from "./tileCache";

const size = { width: 800, height: 600 };
const cam: Camera = { centerX: 1000, centerY: 500, zoom: 2, rotation: 0, flipX: false };

function close(a: number, b: number, eps = 1e-6) {
  expect(Math.abs(a - b)).toBeLessThan(eps);
}

describe("camera", () => {
  it("matches the rust view maths", () => {
    // the same cases as `screen_and_document_coordinates_round_trip` in view.rs
    expect(toScreen(cam, size, 1000, 500)).toEqual([400, 300]);
    for (const c of [
      { ...cam, zoom: 1 },
      { ...cam, zoom: 0.3, rotation: 37 },
      { ...cam, zoom: 4, rotation: -90, flipX: true },
    ]) {
      const [sx, sy] = toScreen(c, size, 1234.5, 321.25);
      const [x, y] = toDocument(c, size, sx, sy);
      close(x, 1234.5);
      close(y, 321.25);
    }
  });

  it("zooms about the pointer", () => {
    const next = zoomAt({ ...cam, rotation: 30 }, size, 100, 80, 3);
    const before = toDocument({ ...cam, rotation: 30 }, size, 100, 80);
    const after = toDocument(next, size, 100, 80);
    close(before[0], after[0]);
    close(before[1], after[1]);
    expect(next.zoom).toBe(6);
  });

  it("pans with the drag whatever the rotation and flip", () => {
    for (const c of [cam, { ...cam, rotation: 73 }, { ...cam, rotation: -20, flipX: true }]) {
      const [x, y] = toDocument(c, size, 300, 200);
      const moved = panBy(c, 25, -40);
      const [nx, ny] = toDocument(moved, size, 325, 160);
      close(x, nx);
      close(y, ny);
    }
  });

  it("rotates about the pointer and wraps angles", () => {
    const next = rotateAround(cam, size, 650, 120, 200);
    expect(next.rotation).toBe(-160);
    const [x, y] = toDocument(cam, size, 650, 120);
    const [nx, ny] = toDocument(next, size, 650, 120);
    close(x, nx);
    close(y, ny);
  });

  it("flips around the centre", () => {
    const f = flip(cam);
    const [sx] = toScreen(f, size, 1010, 500);
    expect(sx).toBe(400 - 20);
  });

  it("fits big pictures and never enlarges small ones", () => {
    const big = fit({ width: 4000, height: 2000 }, size, 0);
    expect(big.zoom).toBe(0.2);
    expect(fit({ width: 100, height: 50 }, size).zoom).toBe(1);
  });

  it("steps through photoshop's zoom levels", () => {
    expect(stepZoom(1, 1)).toBe(2);
    expect(stepZoom(1, -1)).toBeCloseTo(2 / 3);
    expect(stepZoom(0.7, -1)).toBeCloseTo(2 / 3);
    expect(stepZoom(256, 1)).toBe(256);
  });

  it("picks the same mip level as rust", () => {
    expect(levelForZoom(8)).toBe(0);
    expect(levelForZoom(0.6)).toBe(0);
    expect(levelForZoom(0.5)).toBe(1);
    expect(levelForZoom(0.3)).toBe(1);
    expect(levelForZoom(0.01)).toBe(6);
  });

  it("builds a clip matrix that agrees with toScreen", () => {
    const c = { ...cam, rotation: 25, flipX: true, zoom: 0.75 };
    const [m0 = 0, m1 = 0, , m3 = 0, m4 = 0, , m6 = 0, m7 = 0] = clipMatrix(c, size);
    const [x, y] = [1100, 430];
    const cx = m0 * x + m3 * y + m6;
    const cy = m1 * x + m4 * y + m7;
    const [sx, sy] = toScreen(c, size, x, y);
    close(cx, (sx / size.width) * 2 - 1, 1e-5);
    close(cy, 1 - (sy / size.height) * 2, 1e-5);
  });

  it("labels zoom the way people say it", () => {
    expect(zoomLabel(1)).toBe("100%");
    expect(zoomLabel(16)).toBe("1600%");
    expect(zoomLabel(1 / 3)).toBe("33.3%");
    expect(zoomLabel(0.5)).toBe("50%");
    expect(zoomLabel(1 / 64)).toBe("1.56%");
  });
});

describe("frames", () => {
  it("round trip through the binary format", () => {
    const pixels = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const buffer = packFrame({
      tiles: [{ id: { level: 2, col: -1, row: 3 }, rect: { x: -257, y: 767, w: 2, h: 1 }, pixels }],
      dropped: [{ level: 0, col: 5, row: 6 }],
      pending: 7,
      level: 2,
    });
    const frame = parseFrame(buffer);
    expect(frame.pending).toBe(7);
    expect(frame.level).toBe(2);
    expect(frame.dropped).toEqual([{ level: 0, col: 5, row: 6 }]);
    expect(frame.tiles[0]?.rect).toEqual({ x: -257, y: 767, w: 2, h: 1 });
    expect([...(frame.tiles[0]?.pixels ?? [])]).toEqual([...pixels]);
  });

  it("refuses anything else", () => {
    expect(() => parseFrame(new ArrayBuffer(4))).toThrow(/cut short/);
    expect(() => parseFrame(new TextEncoder().encode("NOPE0000000000000000").buffer)).toThrow(
      /format/,
    );
  });
});

describe("tile cache", () => {
  const id = (col: number) => ({ level: 0, col, row: 0 });
  const rect = { x: 0, y: 0, w: 1, h: 1 };

  it("evicts the least recently used tiles and reports them", () => {
    const released: string[] = [];
    const cache = new TileCache<string>(100, (t) => released.push(t));
    cache.put(id(1), rect, "a", 40, new Set());
    cache.put(id(2), rect, "b", 40, new Set());
    cache.touch(id(1));
    const evicted = cache.put(id(3), rect, "c", 40, new Set());
    expect(evicted).toEqual([id(2)]);
    expect(released).toEqual(["b"]);
    expect(cache.size).toBe(2);
  });

  it("never evicts tiles that are on screen", () => {
    const cache = new TileCache<string>(50, () => undefined);
    cache.put(id(1), rect, "a", 40, new Set());
    const evicted = cache.put(id(2), rect, "b", 40, new Set([tileKey(id(1)), tileKey(id(2))]));
    expect(evicted).toEqual([]);
    expect(cache.usedBytes).toBe(80);
  });

  it("draws coarse levels first", () => {
    const cache = new TileCache<string>(1000, () => undefined);
    cache.put({ level: 0, col: 0, row: 0 }, rect, "fine", 1, new Set());
    cache.put({ level: 3, col: 0, row: 0 }, rect, "coarse", 1, new Set());
    expect(cache.drawOrder().map((t) => t.texture)).toEqual(["coarse", "fine"]);
  });
});
