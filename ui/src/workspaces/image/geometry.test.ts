import { describe, expect, it } from "vitest";

import type { LayerState } from "../../bindings/LayerState";
import {
  combineFor,
  cycle,
  fromHex,
  hsvToRgb,
  layerRows,
  marqueeBox,
  parseThumbnails,
  rgbToHsv,
  snapAngle,
  snapBox,
  stepHardness,
  stepSize,
  toHex,
} from "./geometry";

function layer(id: number, name: string, extra: Partial<LayerState> = {}): LayerState {
  return {
    id,
    name,
    visible: true,
    opacity: 1,
    fill: 1,
    blend: "normal",
    passThrough: false,
    locks: { transparency: false, pixels: false, position: false, all: false },
    label: "none",
    clipped: false,
    origin: null,
    group: false,
    expanded: false,
    children: [],
    pixels: null,
    mask: null,
    bounds: { x: 0, y: 0, w: 0, h: 0 },
    ...extra,
  };
}

describe("marquee", () => {
  it("draws boxes from a corner or the centre, square with shift", () => {
    expect(marqueeBox([10, 10], [4, 30], false, false)).toEqual({ x: 4, y: 10, w: 6, h: 20 });
    expect(marqueeBox([10, 10], [16, 30], true, false)).toEqual({ x: 10, y: 10, w: 20, h: 20 });
    expect(marqueeBox([10, 10], [16, 13], false, true)).toEqual({ x: 4, y: 7, w: 12, h: 6 });
    expect(snapBox({ x: 1.4, y: 2.6, w: 3.3, h: 1 })).toEqual({ x: 1, y: 3, w: 4, h: 1 });
  });

  it("modifier keys pick how selections combine", () => {
    expect(combineFor("replace", false, false)).toBe("replace");
    expect(combineFor("replace", true, false)).toBe("add");
    expect(combineFor("add", false, true)).toBe("subtract");
    expect(combineFor("replace", true, true)).toBe("intersect");
  });
});

describe("brush keys", () => {
  it("steps sizes like photoshop and stays in range", () => {
    expect(stepSize(5, 1)).toBe(6);
    expect(stepSize(10, 1)).toBe(20);
    expect(stepSize(20, -1)).toBe(10);
    expect(stepSize(10, -1)).toBe(9);
    expect(stepSize(1, -1)).toBe(1);
    expect(stepSize(150, 1)).toBe(175);
    expect(stepHardness(0.8, 1)).toBe(1);
    expect(stepHardness(0, -1)).toBe(0);
  });

  it("snaps lines to 45 degrees", () => {
    const [x, y] = snapAngle([0, 0], [10, 1]);
    expect(Math.round(x)).toBe(10);
    expect(Math.round(y)).toBe(0);
    const [dx, dy] = snapAngle([0, 0], [10, 9]);
    expect(Math.abs(dx - dy)).toBeLessThan(1e-9);
  });

  it("cycles tool variants", () => {
    expect(cycle(["rect", "ellipse"], "rect")).toBe("ellipse");
    expect(cycle(["rect", "ellipse"], "ellipse")).toBe("rect");
  });
});

describe("colours", () => {
  it("round trips hex and hsv", () => {
    expect(toHex([1, 0.5, 0])).toBe("#ff8000");
    expect(fromHex("#f80")).toEqual([1, 0x88 / 255, 0]);
    expect(fromHex("nope")).toBeNull();
    const c: [number, number, number] = [0.2, 0.6, 0.4];
    const back = hsvToRgb(...rgbToHsv(c));
    back.forEach((v, i) => {
      expect(v).toBeCloseTo(c[i] ?? 0, 6);
    });
  });
});

describe("layer rows", () => {
  const layers = [
    layer(1, "Background"),
    layer(4, "Group 1", {
      group: true,
      expanded: true,
      children: [layer(2, "Sky", { label: "blue" }), layer(3, "Clouds", { visible: false })],
    }),
    layer(5, "Text", { origin: "text" }),
  ];

  it("lists the top of the stack first with children under their group", () => {
    const rows = layerRows(layers);
    expect(rows.map((r) => r.layer.name)).toEqual([
      "Text",
      "Group 1",
      "Clouds",
      "Sky",
      "Background",
    ]);
    expect(rows.find((r) => r.layer.id === 2)).toMatchObject({ depth: 1, parent: 4, index: 0 });
  });

  it("hides children of closed groups unless searching", () => {
    const closed = layers.map((l) => (l.group ? { ...l, expanded: false } : l));
    expect(layerRows(closed)).toHaveLength(3);
    expect(layerRows(closed, "sky").map((r) => r.layer.id)).toEqual([2]);
    expect(layerRows(layers, "hidden").map((r) => r.layer.id)).toEqual([3]);
    expect(layerRows(layers, "blue").map((r) => r.layer.id)).toEqual([2]);
    expect(layerRows(layers, "text").map((r) => r.layer.id)).toEqual([5]);
  });
});

describe("thumbnails", () => {
  it("reads the packed format", () => {
    const buf = new ArrayBuffer(13 + 2 * 1 * 4);
    const v = new DataView(buf);
    v.setUint32(0, 7, true);
    v.setUint8(4, 1);
    v.setUint32(5, 2, true);
    v.setUint32(9, 1, true);
    new Uint8Array(buf, 13).set([1, 2, 3, 4, 5, 6, 7, 8]);
    const thumbs = parseThumbnails(buf);
    expect(thumbs.get("7m")?.width).toBe(2);
    expect(Array.from(thumbs.get("7m")?.pixels ?? [])).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });
});
