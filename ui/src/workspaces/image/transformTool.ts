import type { ImageState } from "../../bindings/ImageState";
import type { TransformTarget } from "../../bindings/TransformTarget";
import type { ViewportContext, ViewportTool } from "../../components/viewport/tool";
import { ipc } from "../../lib/ipc";
import type { Snapper } from "./aids";
import { type CropSession, type TransformSession, useSessions } from "./session";
import {
  type Box,
  type CropBox,
  type Pt,
  type Quad,
  add,
  cropQuad,
  dragCorner,
  dragEdge,
  fitInside,
  isConvex,
  len,
  mapPoint,
  moveQuad,
  perspectiveSize,
  rectQuad,
  rotateAbout,
  rotateQuad,
  sub,
  warpAt,
} from "./transformMath";

// free transform and crop. both keep what they would do in the session store and draw it
// on the viewport's overlay; free transform also shows the pixels through rust's preview.
// enter (or a double click) commits one command, escape throws the session away.

const ACCENT = "#ffb547";
const HANDLE = 4;

/** starts a free transform of the layer (or its selected pixels) or of the selection */
export async function beginTransform(
  doc: string,
  target: TransformTarget,
  fail: (err: unknown) => void,
): Promise<void> {
  try {
    const [x, y, w, h] = await ipc.imageTransformBox(doc, target);
    const source: Box = { x, y, w, h };
    useSessions.getState().setTransform({
      doc,
      target,
      source,
      corners: rectQuad(source),
      warp: null,
      mode: "scale",
      interpolation: "bicubic",
      pivot: [x + w / 2, y + h / 2],
    });
  } catch (err) {
    fail(err);
  }
}

export function transformSpec(t: TransformSession) {
  return {
    source: [t.source.x, t.source.y, t.source.w, t.source.h] as [number, number, number, number],
    corners: t.corners,
    warp: t.warp,
    sourceQuad: null,
    interpolation: t.interpolation,
  };
}

/** commits the transform as one undoable step */
export async function commitTransform(fail: (err: unknown) => void): Promise<void> {
  const t = useSessions.getState().transform;
  if (!t) return;
  useSessions.getState().setTransform(null);
  const untouched =
    t.warp === null && t.corners.every((c, i) => len(sub(c, rectQuad(t.source)[i] as Pt)) < 1e-6);
  try {
    if (untouched) {
      await ipc.imagePreview(t.doc, {
        offset: null,
        showMask: null,
        transform: null,
        filter: null,
      });
      return;
    }
    await ipc.imageTransform(t.doc, { target: t.target, spec: transformSpec(t) });
  } catch (err) {
    fail(err);
  }
}

export function cancelTransform() {
  const t = useSessions.getState().transform;
  if (!t) return;
  useSessions.getState().setTransform(null);
  void ipc
    .imagePreview(t.doc, { offset: null, showMask: null, transform: null, filter: null })
    .catch(() => undefined);
}

function view(cx: ViewportContext, p: Pt): Pt {
  return cx.toView(p[0], p[1]);
}

function square(g: CanvasRenderingContext2D, cx: ViewportContext, p: Pt) {
  const d = cx.dpr();
  const [x, y] = view(cx, p);
  g.fillStyle = "#fff";
  g.strokeStyle = "rgba(0,0,0,0.8)";
  g.lineWidth = d;
  g.fillRect(x - HANDLE * d, y - HANDLE * d, HANDLE * 2 * d, HANDLE * 2 * d);
  g.strokeRect(x - HANDLE * d, y - HANDLE * d, HANDLE * 2 * d, HANDLE * 2 * d);
}

function outline(
  g: CanvasRenderingContext2D,
  cx: ViewportContext,
  pts: readonly Pt[],
  close: boolean,
) {
  g.beginPath();
  pts.forEach((p, i) => {
    const [x, y] = view(cx, p);
    if (i === 0) g.moveTo(x, y);
    else g.lineTo(x, y);
  });
  if (close) g.closePath();
}

function hairline(g: CanvasRenderingContext2D, cx: ViewportContext, colour = ACCENT) {
  g.strokeStyle = "rgba(0,0,0,0.6)";
  g.lineWidth = cx.dpr() * 2.5;
  g.stroke();
  g.strokeStyle = colour;
  g.lineWidth = cx.dpr();
  g.stroke();
}

function insideQuad(q: readonly Pt[], p: Pt): boolean {
  let inside = false;
  for (let i = 0, j = q.length - 1; i < q.length; j = i++) {
    const a = q[i] as Pt;
    const b = q[j] as Pt;
    if (
      a[1] > p[1] !== b[1] > p[1] &&
      p[0] < ((b[0] - a[0]) * (p[1] - a[1])) / (b[1] - a[1]) + a[0]
    ) {
      inside = !inside;
    }
  }
  return inside;
}

function edgeMids(q: Quad): Pt[] {
  return [0, 1, 2, 3].map((i) => {
    const a = q[i] as Pt;
    const b = q[(i + 1) % 4] as Pt;
    return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] as Pt;
  });
}

function nearest(cx: ViewportContext, pts: readonly Pt[], at: Pt): number {
  const tol = (HANDLE + 4) * cx.dpr();
  let best = -1;
  let bestD = Infinity;
  pts.forEach((p, i) => {
    const [x, y] = view(cx, p);
    const d = Math.hypot(x - at[0], y - at[1]);
    if (d <= tol && d < bestD) {
      best = i;
      bestD = d;
    }
  });
  return best;
}

function angleAt(pivot: Pt, p: Pt): number {
  return (Math.atan2(p[1] - pivot[1], p[0] - pivot[0]) * 180) / Math.PI;
}

/** draws the patch's grid: its border and the lines at a third and two thirds */
function drawWarp(g: CanvasRenderingContext2D, cx: ViewportContext, warp: readonly Pt[]) {
  const steps = 24;
  for (const fixed of [0, 1 / 3, 2 / 3, 1]) {
    for (const along of [true, false]) {
      const pts: Pt[] = [];
      for (let i = 0; i <= steps; i++) {
        const t = i / steps;
        pts.push(along ? warpAt(warp, t, fixed) : warpAt(warp, fixed, t));
      }
      outline(g, cx, pts, false);
      hairline(g, cx);
    }
  }
}

/** the selection's outline, carried along by the transform */
function movedAnts(t: TransformSession, ants: readonly (readonly Pt[])[]): Pt[][] {
  return ants.map((loop) => loop.map((p) => mapPoint(t.source, t.corners, t.warp, p) ?? p));
}

export interface TransformToolEnv {
  ants: readonly (readonly Pt[])[];
  snap: Snapper;
  fail: (err: unknown) => void;
  drawAnts: (
    g: CanvasRenderingContext2D,
    cx: ViewportContext,
    loops: readonly (readonly Pt[])[],
    time: number,
  ) => boolean;
}

type Drag =
  | { kind: "move"; from: Pt; start: Quad; warp: Pt[] | null; pivot: Pt }
  | { kind: "rotate"; from: number; start: Quad; warp: Pt[] | null; pivot: Pt }
  | { kind: "corner"; k: number; start: Quad }
  | { kind: "edge"; e: number; start: Quad }
  | { kind: "warp"; k: number; from: Pt; start: Pt[] }
  | { kind: "pivot" };

export function transformTool(env: TransformToolEnv): ViewportTool {
  let drag: Drag | null = null;
  const get = () => useSessions.getState().transform;
  const patch = (p: Partial<TransformSession>) => {
    useSessions.getState().patchTransform(p);
  };
  return {
    cursor: "default",
    down: (event, cx) => {
      const t = get();
      if (!t || event.button !== 0) return false;
      const at = cx.toView(...cx.toDoc(event.clientX, event.clientY));
      const p = cx.toDoc(event.clientX, event.clientY);
      if (t.warp) {
        const k = nearest(cx, t.warp, at);
        if (k >= 0) {
          drag = { kind: "warp", k, from: p, start: t.warp.map((w) => [w[0], w[1]] as Pt) };
          return true;
        }
        const hull = [0, 1, 2, 3].map((i) => t.warp?.[[0, 3, 15, 12][i] ?? 0] ?? [0, 0]) as Pt[];
        if (insideQuad(hull, p)) {
          drag = { kind: "move", from: p, start: t.corners, warp: t.warp, pivot: t.pivot };
          return true;
        }
        return true;
      }
      if (nearest(cx, [t.pivot], at) === 0) {
        drag = { kind: "pivot" };
        return true;
      }
      const k = nearest(cx, t.corners, at);
      if (k >= 0) {
        drag = { kind: "corner", k, start: t.corners };
        return true;
      }
      const e = nearest(cx, edgeMids(t.corners), at);
      if (e >= 0) {
        drag = { kind: "edge", e, start: t.corners };
        return true;
      }
      if (insideQuad(t.corners, p) && t.mode !== "rotate") {
        drag = { kind: "move", from: p, start: t.corners, warp: null, pivot: t.pivot };
        return true;
      }
      drag = {
        kind: "rotate",
        from: angleAt(t.pivot, p),
        start: t.corners,
        warp: null,
        pivot: t.pivot,
      };
      return true;
    },
    move: (event, cx) => {
      const t = get();
      if (!t || !drag) return;
      const p = cx.toDoc(event.clientX, event.clientY);
      const ctrl = event.ctrlKey || event.metaKey;
      switch (drag.kind) {
        case "pivot":
          patch({ pivot: env.snap.point(p, cx, { ctrlKey: false, metaKey: false }) });
          break;
        case "move": {
          const xs = drag.start.map((c) => c[0]);
          const ys = drag.start.map((c) => c[1]);
          const box = {
            x: Math.min(...xs),
            y: Math.min(...ys),
            w: Math.max(...xs) - Math.min(...xs),
            h: Math.max(...ys) - Math.min(...ys),
          };
          const [dx, dy] = env.snap.move(box, p[0] - drag.from[0], p[1] - drag.from[1], cx, event);
          const d: Pt = [dx, dy];
          patch({
            corners: moveQuad(drag.start, d),
            warp: drag.warp?.map((w) => add(w, d)) ?? null,
            pivot: add(drag.pivot, d),
          });
          break;
        }
        case "rotate": {
          let deg = angleAt(drag.pivot, p) - drag.from;
          if (event.shiftKey) {
            const now = Math.atan2(
              drag.start[1][1] - drag.start[0][1],
              drag.start[1][0] - drag.start[0][0],
            );
            const total = (now * 180) / Math.PI + deg;
            deg = Math.round(total / 15) * 15 - (now * 180) / Math.PI;
          }
          patch({ corners: rotateQuad(drag.start, drag.pivot, deg) });
          break;
        }
        case "corner": {
          const mode =
            ctrl && event.altKey && event.shiftKey
              ? "perspective"
              : ctrl
                ? "distort"
                : t.mode === "rotate" || t.mode === "skew"
                  ? "scale"
                  : t.mode;
          const target = env.snap.point(p, cx, event);
          const next = dragCorner(drag.start, drag.k, target, mode, !event.shiftKey, event.altKey);
          if (isConvex(next)) patch({ corners: next });
          break;
        }
        case "edge": {
          const mode = ctrl || t.mode === "skew" ? "skew" : t.mode === "distort" ? "skew" : "scale";
          const next = dragEdge(
            drag.start,
            drag.e,
            env.snap.point(p, cx, event),
            mode,
            event.altKey,
          );
          if (isConvex(next)) patch({ corners: next });
          break;
        }
        case "warp": {
          const d = sub(p, drag.from);
          const k = drag.k;
          const warp = drag.start.map((w, i) => (i === k ? add(w, d) : w));
          patch({ warp });
          break;
        }
      }
      cx.redraw();
    },
    up: () => {
      drag = null;
      env.snap.clear();
    },
    cancel: () => {
      drag = null;
      env.snap.clear();
    },
    doubleClick: (event, cx) => {
      const t = get();
      if (!t) return false;
      if (insideQuad(t.corners, cx.toDoc(event.clientX, event.clientY))) {
        void commitTransform(env.fail);
        return true;
      }
      return false;
    },
    key: (event) => {
      const t = get();
      if (!t) return false;
      if (event.key === "Enter") {
        void commitTransform(env.fail);
        return true;
      }
      if (event.key === "Escape") {
        cancelTransform();
        return true;
      }
      const step = event.shiftKey ? 10 : 1;
      const moves: Record<string, Pt> = {
        ArrowLeft: [-step, 0],
        ArrowRight: [step, 0],
        ArrowUp: [0, -step],
        ArrowDown: [0, step],
      };
      const m = moves[event.key];
      if (!m) return false;
      patch({
        corners: moveQuad(t.corners, m),
        warp: t.warp?.map((w) => add(w, m)) ?? null,
        pivot: add(t.pivot, m),
      });
      return true;
    },
    draw: (g, cx, time) => {
      const t = get();
      if (!t) return false;
      const animating = env.drawAnts(g, cx, movedAnts(t, env.ants), time);
      if (t.warp) {
        drawWarp(g, cx, t.warp);
        // the handles' arms, then the points
        g.beginPath();
        for (const [a, b] of [
          [0, 1],
          [2, 3],
          [0, 4],
          [12, 8],
          [3, 7],
          [15, 11],
          [12, 13],
          [15, 14],
        ] as const) {
          const pa = view(cx, t.warp[a] as Pt);
          const pb = view(cx, t.warp[b] as Pt);
          g.moveTo(pa[0], pa[1]);
          g.lineTo(pb[0], pb[1]);
        }
        g.strokeStyle = "rgba(255,255,255,0.6)";
        g.lineWidth = cx.dpr();
        g.stroke();
        for (const p of t.warp) square(g, cx, p);
        return animating;
      }
      outline(g, cx, t.corners, true);
      hairline(g, cx);
      for (const p of [...t.corners, ...edgeMids(t.corners)]) square(g, cx, p);
      const [px, py] = view(cx, t.pivot);
      const d = cx.dpr();
      g.beginPath();
      g.arc(px, py, 5 * d, 0, Math.PI * 2);
      g.moveTo(px - 8 * d, py);
      g.lineTo(px + 8 * d, py);
      g.moveTo(px, py - 8 * d);
      g.lineTo(px, py + 8 * d);
      hairline(g, cx, "#fff");
      return animating;
    },
  };
}

/** a fresh crop over the whole canvas */
export function startCrop(doc: string, state: ImageState, perspective: boolean): CropSession {
  const box: CropBox = {
    cx: state.width / 2,
    cy: state.height / 2,
    w: state.width,
    h: state.height,
    angle: 0,
  };
  return {
    doc,
    box,
    quad: perspective ? cropQuad(box) : null,
    aspect: 0,
    deleteOutside: true,
    straighten: false,
  };
}

export async function commitCrop(fail: (err: unknown) => void): Promise<void> {
  const c = useSessions.getState().crop;
  if (!c) return;
  useSessions.getState().setCrop(null);
  let params: unknown;
  if (c.quad) {
    const [w, h] = perspectiveSize(c.quad);
    params = { rect: [0, 0, w, h], perspective: c.quad, deleteOutside: c.deleteOutside };
  } else {
    const b = c.box;
    params = {
      rect: [b.cx - b.w / 2, b.cy - b.h / 2, b.w, b.h],
      angle: b.angle,
      deleteOutside: c.deleteOutside,
    };
  }
  await ipc.imageCommand(c.doc, "image.crop", params).catch(fail);
}

/** the box in its own turned frame */
function toLocal(b: CropBox, p: Pt): Pt {
  return rotateAbout(p, [b.cx, b.cy], -b.angle);
}

function fromLocal(b: CropBox, p: Pt): Pt {
  return rotateAbout(p, [b.cx, b.cy], b.angle);
}

/** a box from two opposite local corners, turned back */
function boxFrom(b: CropBox, a: Pt, z: Pt): CropBox {
  const mid = fromLocal(b, [(a[0] + z[0]) / 2, (a[1] + z[1]) / 2]);
  return {
    ...b,
    cx: mid[0],
    cy: mid[1],
    w: Math.max(1, Math.abs(z[0] - a[0])),
    h: Math.max(1, Math.abs(z[1] - a[1])),
  };
}

export interface CropToolEnv {
  state: ImageState;
  snap: Snapper;
  fail: (err: unknown) => void;
}

type CropDrag =
  | { kind: "move"; from: Pt; box: CropBox; quad: Quad | null }
  | { kind: "rotate"; from: number; box: CropBox }
  | { kind: "corner"; k: number; box: CropBox; quad: Quad | null }
  | { kind: "edge"; e: number; box: CropBox }
  | { kind: "straighten"; from: Pt; to: Pt };

export function cropTool(env: CropToolEnv): ViewportTool {
  let drag: CropDrag | null = null;
  const get = () => useSessions.getState().crop;
  const patch = (p: Partial<CropSession>) => {
    useSessions.getState().patchCrop(p);
  };
  const ratio = (c: CropSession) => (c.aspect < 0 ? env.state.width / env.state.height : c.aspect);
  return {
    cursor: "crosshair",
    down: (event, cx) => {
      const c = get();
      if (!c || event.button !== 0) return false;
      const p = cx.toDoc(event.clientX, event.clientY);
      const at = cx.toView(p[0], p[1]);
      if (c.straighten) {
        drag = { kind: "straighten", from: p, to: p };
        return true;
      }
      const quad = c.quad ?? cropQuad(c.box);
      const k = nearest(cx, quad, at);
      if (k >= 0) {
        drag = { kind: "corner", k, box: c.box, quad: c.quad };
        return true;
      }
      if (!c.quad) {
        const e = nearest(cx, edgeMids(quad), at);
        if (e >= 0) {
          drag = { kind: "edge", e, box: c.box };
          return true;
        }
      }
      if (insideQuad(quad, p)) {
        drag = { kind: "move", from: p, box: c.box, quad: c.quad };
        return true;
      }
      if (!c.quad) {
        drag = { kind: "rotate", from: angleAt([c.box.cx, c.box.cy], p), box: c.box };
        return true;
      }
      return true;
    },
    move: (event, cx) => {
      const c = get();
      if (!c || !drag) return;
      const p = cx.toDoc(event.clientX, event.clientY);
      switch (drag.kind) {
        case "straighten":
          drag.to = p;
          break;
        case "move": {
          const q = drag.quad ?? cropQuad(drag.box);
          const xs = q.map((v) => v[0]);
          const ys = q.map((v) => v[1]);
          const box = {
            x: Math.min(...xs),
            y: Math.min(...ys),
            w: Math.max(...xs) - Math.min(...xs),
            h: Math.max(...ys) - Math.min(...ys),
          };
          const [dx, dy] = env.snap.move(box, p[0] - drag.from[0], p[1] - drag.from[1], cx, event);
          if (drag.quad) patch({ quad: moveQuad(drag.quad, [dx, dy]) });
          else patch({ box: { ...drag.box, cx: drag.box.cx + dx, cy: drag.box.cy + dy } });
          break;
        }
        case "rotate": {
          let deg = drag.box.angle + angleAt([drag.box.cx, drag.box.cy], p) - drag.from;
          if (event.shiftKey) deg = Math.round(deg / 15) * 15;
          patch({ box: { ...drag.box, angle: deg } });
          break;
        }
        case "corner": {
          const target = env.snap.point(p, cx, event);
          if (drag.quad) {
            const k = drag.k;
            const q = drag.quad.map((v, i) => (i === k ? target : v)) as Quad;
            if (isConvex(q)) patch({ quad: q });
            break;
          }
          const b = drag.box;
          const corners = cropQuad(b).map((v) => toLocal(b, v));
          const anchor = corners[(drag.k + 2) % 4] as Pt;
          let moved = toLocal(b, target);
          const r = event.shiftKey && c.aspect === 0 ? b.w / b.h : ratio(c);
          if (r > 0) {
            const dx = moved[0] - anchor[0];
            const dy = moved[1] - anchor[1];
            const w = Math.max(Math.abs(dx), Math.abs(dy) * r);
            moved = [anchor[0] + Math.sign(dx || 1) * w, anchor[1] + (Math.sign(dy || 1) * w) / r];
          }
          patch({ box: boxFrom(b, anchor, moved) });
          break;
        }
        case "edge": {
          const b = drag.box;
          const l = toLocal(b, env.snap.point(p, cx, event));
          const hw = b.w / 2;
          const hh = b.h / 2;
          let a: Pt = [b.cx - hw, b.cy - hh];
          let z: Pt = [b.cx + hw, b.cy + hh];
          if (drag.e === 0) a = [a[0], l[1]];
          if (drag.e === 1) z = [l[0], z[1]];
          if (drag.e === 2) z = [z[0], l[1]];
          if (drag.e === 3) a = [l[0], a[1]];
          const r = ratio(c);
          if (r > 0) {
            // the other side follows to keep the shape, about the middle
            if (drag.e % 2 === 0) {
              const w = Math.abs(z[1] - a[1]) * r;
              a = [b.cx - w / 2, a[1]];
              z = [b.cx + w / 2, z[1]];
            } else {
              const h = Math.abs(z[0] - a[0]) / r;
              a = [a[0], b.cy - h / 2];
              z = [z[0], b.cy + h / 2];
            }
          }
          patch({ box: boxFrom(b, a, z) });
          break;
        }
      }
      cx.redraw();
    },
    up: () => {
      const c = get();
      if (c && drag?.kind === "straighten") {
        const d = sub(drag.to, drag.from);
        if (len(d) > 2) {
          const deg = levelAngle(drag.from, drag.to);
          const box = fitInside(
            {
              cx: env.state.width / 2,
              cy: env.state.height / 2,
              w: env.state.width,
              h: env.state.height,
              angle: deg,
            },
            env.state.width,
            env.state.height,
          );
          patch({ box, straighten: false, quad: null });
        }
      }
      drag = null;
      env.snap.clear();
    },
    cancel: () => {
      drag = null;
      env.snap.clear();
    },
    doubleClick: (event, cx) => {
      const c = get();
      if (!c) return false;
      const q = c.quad ?? cropQuad(c.box);
      if (insideQuad(q, cx.toDoc(event.clientX, event.clientY))) {
        void commitCrop(env.fail);
        return true;
      }
      return false;
    },
    key: (event) => {
      const c = get();
      if (!c) return false;
      if (event.key === "Enter") {
        void commitCrop(env.fail);
        return true;
      }
      if (event.key === "Escape") {
        useSessions.getState().setCrop(startCrop(c.doc, env.state, c.quad !== null));
        return true;
      }
      return false;
    },
    draw: (g, cx) => {
      const c = get();
      if (!c) return false;
      const q = c.quad ?? cropQuad(c.box);
      const size = cx.size();
      const d = cx.dpr();
      // darken what will be cut away
      g.beginPath();
      g.rect(0, 0, size.width, size.height);
      const pts = q.map((p) => view(cx, p));
      pts.forEach(([x, y], i) => {
        if (i === 0) g.moveTo(x, y);
        else g.lineTo(x, y);
      });
      g.closePath();
      g.fillStyle = "rgba(0,0,0,0.55)";
      g.fill("evenodd");
      outline(g, cx, q, true);
      hairline(g, cx, "#fff");
      if (!c.quad) {
        // thirds
        const b = c.box;
        const lines: [Pt, Pt][] = [];
        for (const f of [1 / 3, 2 / 3]) {
          lines.push([
            fromLocal(b, [b.cx - b.w / 2 + b.w * f, b.cy - b.h / 2]),
            fromLocal(b, [b.cx - b.w / 2 + b.w * f, b.cy + b.h / 2]),
          ]);
          lines.push([
            fromLocal(b, [b.cx - b.w / 2, b.cy - b.h / 2 + b.h * f]),
            fromLocal(b, [b.cx + b.w / 2, b.cy - b.h / 2 + b.h * f]),
          ]);
        }
        g.beginPath();
        for (const [a, z] of lines) {
          const [ax, ay] = view(cx, a);
          const [zx, zy] = view(cx, z);
          g.moveTo(ax, ay);
          g.lineTo(zx, zy);
        }
        g.strokeStyle = "rgba(255,255,255,0.45)";
        g.lineWidth = d;
        g.stroke();
      }
      for (const p of c.quad ? q : [...q, ...edgeMids(q)]) square(g, cx, p);
      const [w, h] = c.quad ? perspectiveSize(c.quad) : [Math.round(c.box.w), Math.round(c.box.h)];
      const top = pts.reduce((m, p) => (p[1] < m[1] ? p : m), pts[0] ?? [0, 0]);
      g.font = `${String(11 * d)}px Inter Variable, sans-serif`;
      const label = `${String(w)} × ${String(h)} px${!c.quad && Math.abs(c.box.angle) > 0.01 ? ` · ${c.box.angle.toFixed(1)}°` : ""}`;
      g.fillStyle = "rgba(0,0,0,0.7)";
      const tw = g.measureText(label).width;
      g.fillRect(top[0] - tw / 2 - 4 * d, top[1] - 22 * d, tw + 8 * d, 16 * d);
      g.fillStyle = "#fff";
      g.fillText(label, top[0] - tw / 2, top[1] - 10 * d);
      if (drag?.kind === "straighten") {
        outline(g, cx, [drag.from, drag.to], false);
        hairline(g, cx);
      }
      return false;
    },
  };
}

/** the turn that makes the line from `a` to `b` level, or upright when it is closer to that */
export function levelAngle(a: Pt, b: Pt): number {
  let deg = (Math.atan2(b[1] - a[1], b[0] - a[0]) * 180) / Math.PI;
  while (deg > 45) deg -= 90;
  while (deg < -45) deg += 90;
  return deg;
}
