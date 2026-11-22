import { describe, expect, it } from "vitest";

import { keepRatio } from "./geometry";
import { levelAngle } from "./transformTool";
import {
  type Quad,
  apply,
  cropQuad,
  dragCorner,
  dragEdge,
  fitInside,
  flipQuad,
  fromAffine,
  isConvex,
  isParallelogram,
  mapPoint,
  perspectiveSize,
  quadToQuad,
  rectQuad,
  rulerStep,
  snapMove,
  toAffine,
  warpAt,
  warpFromQuad,
} from "./transformMath";

const box = { x: 10, y: 20, w: 100, h: 50 };

function close(
  a: readonly number[] | null | undefined,
  b: readonly number[] | undefined,
  eps = 1e-6,
) {
  expect(a).toBeTruthy();
  expect(b).toBeTruthy();
  (a ?? []).forEach((v, i) => {
    expect(Math.abs(v - (b[i] ?? NaN))).toBeLessThan(eps);
  });
}

describe("the corner map", () => {
  it("maps a box onto its corners, like rust", () => {
    const corners: Quad = [
      [0, 0],
      [100, 10],
      [90, 80],
      [5, 60],
    ];
    const m = quadToQuad(rectQuad(box), corners);
    if (!m) throw new Error("no map");
    rectQuad(box).forEach((p, i) => {
      close(apply(m, p), corners[i]);
    });
    // a scale by two about the top left, the same case as the rust test
    const doubled = quadToQuad(rectQuad(box), [
      [10, 20],
      [210, 20],
      [210, 120],
      [10, 120],
    ]);
    if (!doubled) throw new Error("no map");
    close(apply(doubled, [60, 45]), [110, 70]);
  });

  it("refuses a box that folds over itself", () => {
    const bowtie: Quad = [
      [0, 0],
      [100, 100],
      [100, 0],
      [0, 100],
    ];
    expect(quadToQuad(rectQuad(box), bowtie)).toBeNull();
    expect(isConvex(bowtie)).toBe(false);
    expect(isConvex(rectQuad(box))).toBe(true);
  });
});

describe("the warp patch", () => {
  it("starts flat, matching the corners", () => {
    const pts = warpFromQuad(box, rectQuad(box));
    close(warpAt(pts, 0.5, 0.25), [60, 32.5]);
    close(mapPoint(box, rectQuad(box), pts, [35, 45]), [35, 45]);
  });
});

describe("handles", () => {
  const q = rectQuad(box);

  it("scale a corner against the opposite one, keeping the shape", () => {
    const next = dragCorner(q, 2, [210, 120], "scale", true, false);
    close(next[0], [10, 20]);
    close(next[2], [210, 120]);
    // without proportions only the dragged axes follow
    const free = dragCorner(q, 2, [210, 70], "scale", false, false);
    close(free[2], [210, 70]);
  });

  it("move a corner on its own when distorting", () => {
    const next = dragCorner(q, 1, [120, 0], "distort", false, false);
    close(next[1], [120, 0]);
    close(next[0], q[0]);
  });

  it("move a corner and its neighbour apart for perspective", () => {
    const next = dragCorner(q, 0, [0, 20], "perspective", false, false);
    close(next[0], [0, 20]);
    close(next[1], [120, 20]);
  });

  it("scale or skew one edge", () => {
    const wider = dragEdge(q, 1, [150, 40], "scale", false);
    close(wider[1], [150, 20]);
    close(wider[0], q[0]);
    const leaning = dragEdge(q, 0, [70, 20], "skew", false);
    close(leaning[0], [20, 20]);
    close(leaning[3], q[3]);
  });

  it("flip in place", () => {
    const f = flipQuad(q, true);
    close(f[0], q[1]);
    close(f[3], q[2]);
  });
});

describe("numbers for the options bar", () => {
  it("round trip through centre, size, angle and skew", () => {
    const a = { x: 60, y: 45, w: 150, h: 80, angle: 30, skew: 10 };
    const q = fromAffine(box, a);
    expect(isParallelogram(q)).toBe(true);
    const back = toAffine(box, q);
    close([back.x, back.y, back.w, back.h, back.angle, back.skew], [60, 45, 150, 80, 30, 10]);
  });
});

describe("crop", () => {
  it("straightens to the line drawn, level or upright", () => {
    expect(levelAngle([0, 0], [100, 10])).toBeCloseTo(5.71, 2);
    expect(levelAngle([0, 0], [5, 100])).toBeCloseTo(-2.86, 2);
  });

  it("keeps a turned box inside the picture", () => {
    const fitted = fitInside({ cx: 50, cy: 40, w: 100, h: 80, angle: 10 }, 100, 80);
    expect(fitted.w).toBeLessThan(100);
    expect(fitted.w / fitted.h).toBeCloseTo(100 / 80);
    for (const [x, y] of cropQuad(fitted)) {
      expect(x).toBeGreaterThanOrEqual(-1e-6);
      expect(x).toBeLessThanOrEqual(100 + 1e-6);
      expect(y).toBeGreaterThanOrEqual(-1e-6);
      expect(y).toBeLessThanOrEqual(80 + 1e-6);
    }
  });

  it("sizes a perspective crop from its sides", () => {
    expect(
      perspectiveSize([
        [0, 0],
        [100, 0],
        [90, 50],
        [10, 50],
      ]),
    ).toEqual([90, 51]);
  });
});

describe("snapping and rulers", () => {
  it("lands the nearest edge on a line within reach", () => {
    const edges = { xs: [10, 60, 110], ys: [20] };
    const s = snapMove(edges, { xs: [113], ys: [100] }, 0, 0, 4);
    expect(s.dx).toBe(3);
    expect(s.dy).toBe(0);
    expect(s.lines).toEqual([{ vertical: true, at: 113 }]);
  });

  it("spaces ruler numbers so they never crowd", () => {
    expect(rulerStep(1, 50)).toBe(50);
    expect(rulerStep(0.1, 50)).toBe(500);
    expect(rulerStep(10, 50)).toBe(5);
  });

  it("keeps proportions in image size", () => {
    expect(keepRatio(50, 100, 80)).toBe(40);
  });
});
