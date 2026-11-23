import type { Guide } from "../../bindings/Guide";
import type { ImageState } from "../../bindings/ImageState";
import type { LayerState } from "../../bindings/LayerState";
import type { ViewportContext, ViewportTool } from "../../components/viewport/tool";
import { ipc } from "../../lib/ipc";
import { toDocument } from "../../lib/viewport/camera";
import type { ViewAids } from "./session";
import {
  type Box,
  type Pt,
  type Snap,
  type SnapTargets,
  boxEdges,
  rulerStep,
  snapMove,
} from "./transformMath";

// rulers, guides, the grid and smart guides, drawn over any tool on the viewport's overlay.
// guides are part of the document (adding, moving and removing one is a command, so it
// undoes); whether they show, lock or snap is per window, like photoshop's view menu.

/** how wide the rulers are, in css pixels */
export const RULER = 18;
/** how close, in css pixels, a drag must come to something to snap to it */
const SNAP_PX = 6;

const GUIDE = "rgba(0, 220, 255, 0.95)";
const SMART = "rgba(255, 61, 184, 0.95)";

export interface AidsEnv {
  doc: string;
  state: ImageState | null;
  aids: ViewAids;
  /** guides can be dragged only with the move tool, like photoshop */
  canDragGuides: boolean;
  /** the lines the current drag snapped to, drawn until the drag ends */
  lines: { current: Snap["lines"] };
  fail: (err: unknown) => void;
}

/** what tools use to line things up while they drag */
export interface Snapper {
  /** a point snapped to the nearest line on each axis */
  point: (p: Pt, cx: ViewportContext, event: { ctrlKey: boolean; metaKey: boolean }) => Pt;
  /** a move of `box` by (dx, dy), snapped so an edge or middle lands on a line */
  move: (
    box: Box,
    dx: number,
    dy: number,
    cx: ViewportContext,
    event: { ctrlKey: boolean; metaKey: boolean },
  ) => [number, number];
  clear: () => void;
}

function straight(cx: ViewportContext): boolean {
  const cam = cx.camera();
  return Math.abs(cam.rotation) < 1e-6;
}

function visibleLayers(list: readonly LayerState[], skip: readonly number[], out: Box[]) {
  for (const l of list) {
    if (!l.visible || skip.includes(l.id)) continue;
    if (!l.group && l.bounds.w > 0 && l.bounds.h > 0) out.push(l.bounds);
    visibleLayers(l.children, skip, out);
  }
}

/** everything a drag can snap to, given what is switched on */
export function snapTargets(env: AidsEnv, skipLayers: readonly number[]): SnapTargets {
  const s = env.state;
  const a = env.aids;
  const out: SnapTargets = { xs: [], ys: [] };
  if (!s || !a.snap) return out;
  if (a.snapCanvas) {
    out.xs.push(0, s.width / 2, s.width);
    out.ys.push(0, s.height / 2, s.height);
  }
  if (a.snapGuides && a.guides) {
    for (const g of s.guides) (g.vertical ? out.xs : out.ys).push(g.position);
  }
  if (a.snapLayers) {
    const boxes: Box[] = [];
    visibleLayers(s.layers, skipLayers, boxes);
    for (const b of boxes) {
      const e = boxEdges(b);
      out.xs.push(...e.xs);
      out.ys.push(...e.ys);
    }
  }
  return out;
}

export function makeSnapper(env: AidsEnv, skipLayers: readonly number[]): Snapper {
  const targets = snapTargets(env, skipLayers);
  const grid =
    env.aids.snap && env.aids.snapGrid && env.aids.grid
      ? env.aids.gridSize / Math.max(1, env.aids.gridDivisions)
      : 0;
  const tolerance = (cx: ViewportContext) => (SNAP_PX * cx.dpr()) / cx.camera().zoom;
  // the grid line nearest each moving edge joins the targets for that move
  const withGrid = (edges: SnapTargets, dx: number, dy: number): SnapTargets => {
    if (grid <= 0) return targets;
    return {
      xs: [...targets.xs, ...edges.xs.map((x) => Math.round((x + dx) / grid) * grid)],
      ys: [...targets.ys, ...edges.ys.map((y) => Math.round((y + dy) / grid) * grid)],
    };
  };
  const off = (e: { ctrlKey: boolean; metaKey: boolean }) =>
    !env.aids.snap || e.ctrlKey || e.metaKey;
  return {
    point: (p, cx, event) => {
      if (off(event)) {
        env.lines.current = [];
        return p;
      }
      const edges = { xs: [p[0]], ys: [p[1]] };
      const s = snapMove(edges, withGrid(edges, 0, 0), 0, 0, tolerance(cx));
      env.lines.current = s.lines;
      return [p[0] + s.dx, p[1] + s.dy];
    },
    move: (box, dx, dy, cx, event) => {
      if (off(event)) {
        env.lines.current = [];
        return [dx, dy];
      }
      const edges = boxEdges(box);
      const s = snapMove(edges, withGrid(edges, dx, dy), dx, dy, tolerance(cx));
      env.lines.current = s.lines;
      return [s.dx, s.dy];
    },
    clear: () => {
      env.lines.current = [];
    },
  };
}

/** the document rectangle the view shows, roughly, for drawing lines across it */
function viewSpan(cx: ViewportContext): { x0: number; y0: number; x1: number; y1: number } {
  const cam = cx.camera();
  const size = cx.size();
  const pts = [
    toDocument(cam, size, 0, 0),
    toDocument(cam, size, size.width, 0),
    toDocument(cam, size, 0, size.height),
    toDocument(cam, size, size.width, size.height),
  ];
  return {
    x0: Math.min(...pts.map((p) => p[0])),
    y0: Math.min(...pts.map((p) => p[1])),
    x1: Math.max(...pts.map((p) => p[0])),
    y1: Math.max(...pts.map((p) => p[1])),
  };
}

function line(g: CanvasRenderingContext2D, cx: ViewportContext, a: Pt, b: Pt) {
  const [ax, ay] = cx.toView(a[0], a[1]);
  const [bx, by] = cx.toView(b[0], b[1]);
  g.moveTo(ax, ay);
  g.lineTo(bx, by);
}

function drawGrid(g: CanvasRenderingContext2D, cx: ViewportContext, s: ImageState, a: ViewAids) {
  const zoom = cx.camera().zoom;
  const span = viewSpan(cx);
  const x0 = Math.max(0, span.x0);
  const y0 = Math.max(0, span.y0);
  const x1 = Math.min(s.width, span.x1);
  const y1 = Math.min(s.height, span.y1);
  const minor = a.gridSize / Math.max(1, a.gridDivisions);
  const layers: [number, string][] = [];
  if (minor * zoom >= 8 * cx.dpr() && a.gridDivisions > 1)
    layers.push([minor, "rgba(140,140,140,0.22)"]);
  if (a.gridSize * zoom >= 8 * cx.dpr()) layers.push([a.gridSize, "rgba(160,160,160,0.5)"]);
  g.lineWidth = cx.dpr();
  for (const [step, colour] of layers) {
    g.beginPath();
    for (let x = Math.ceil(x0 / step) * step; x <= x1; x += step) line(g, cx, [x, y0], [x, y1]);
    for (let y = Math.ceil(y0 / step) * step; y <= y1; y += step) line(g, cx, [x0, y], [x1, y]);
    g.strokeStyle = colour;
    g.stroke();
  }
}

function drawGuides(
  g: CanvasRenderingContext2D,
  cx: ViewportContext,
  guides: readonly { vertical: boolean; position: number }[],
  colour: string,
) {
  if (guides.length === 0) return;
  const span = viewSpan(cx);
  g.beginPath();
  for (const guide of guides) {
    if (guide.vertical) line(g, cx, [guide.position, span.y0], [guide.position, span.y1]);
    else line(g, cx, [span.x0, guide.position], [span.x1, guide.position]);
  }
  g.lineWidth = cx.dpr();
  g.strokeStyle = colour;
  g.stroke();
}

function drawRulers(g: CanvasRenderingContext2D, cx: ViewportContext) {
  const d = cx.dpr();
  const size = cx.size();
  const band = RULER * d;
  const zoom = cx.camera().zoom;
  const style = getComputedStyle(document.documentElement);
  const bg = style.getPropertyValue("--surface-2").trim() || "#1c1d21";
  const fg = style.getPropertyValue("--fg-2").trim() || "#a6a29a";
  g.fillStyle = bg;
  g.fillRect(0, 0, size.width, band);
  g.fillRect(0, 0, band, size.height);
  const step = rulerStep(zoom, 60 * d);
  const minor = step / 5;
  const span = viewSpan(cx);
  g.strokeStyle = fg;
  g.fillStyle = fg;
  g.lineWidth = d;
  g.font = `${String(9 * d)}px Inter Variable, sans-serif`;
  g.textBaseline = "top";
  g.beginPath();
  for (let x = Math.floor(span.x0 / minor) * minor; x <= span.x1; x += minor) {
    const sx = cx.toView(x, 0)[0];
    if (sx < band) continue;
    const major = Math.abs(x / step - Math.round(x / step)) < 1e-6;
    g.moveTo(sx, band);
    g.lineTo(sx, band - (major ? band * 0.6 : band * 0.25));
    if (major) g.fillText(String(Math.round(x)), sx + 2 * d, 2 * d);
  }
  for (let y = Math.floor(span.y0 / minor) * minor; y <= span.y1; y += minor) {
    const sy = cx.toView(0, y)[1];
    if (sy < band) continue;
    const major = Math.abs(y / step - Math.round(y / step)) < 1e-6;
    g.moveTo(band, sy);
    g.lineTo(band - (major ? band * 0.6 : band * 0.25), sy);
    if (major) {
      g.save();
      g.translate(2 * d, sy + 2 * d);
      g.rotate(Math.PI / 2);
      g.fillText(String(Math.round(y)), 0, -band + 4 * d);
      g.restore();
    }
  }
  g.stroke();
  g.strokeStyle = "rgba(0,0,0,0.35)";
  g.beginPath();
  g.moveTo(0, band);
  g.lineTo(size.width, band);
  g.moveTo(band, 0);
  g.lineTo(band, size.height);
  g.stroke();
}

/** where on the view a pointer is, in device pixels */
function local(event: PointerEvent | MouseEvent, cx: ViewportContext): Pt {
  // toDoc then toView gives device pixels relative to the canvas whatever its offset
  const [x, y] = cx.toDoc(event.clientX, event.clientY);
  return cx.toView(x, y);
}

function nearGuide(guides: readonly Guide[], cx: ViewportContext, p: Pt): Guide | null {
  const tol = 4 * cx.dpr();
  let best: { g: Guide; d: number } | null = null;
  for (const guide of guides) {
    const at = guide.vertical
      ? cx.toView(guide.position, 0)[0] - p[0]
      : cx.toView(0, guide.position)[1] - p[1];
    const d = Math.abs(at);
    if (d <= tol && (!best || d < best.d)) best = { g: guide, d };
  }
  return best?.g ?? null;
}

/**
 * the active tool with rulers, guides, the grid and smart guides around it. presses on
 * a ruler drag out a new guide; with the move tool, presses on a guide move it, and
 * dropping it back on a ruler deletes it.
 */
export function withAids(tool: ViewportTool, env: AidsEnv): ViewportTool {
  const s = env.state;
  const a = env.aids;
  let drag: { id: number | null; vertical: boolean; position: number } | null = null;
  const rulersOn = () => a.rulers;
  const inRuler = (p: Pt, cx: ViewportContext): "top" | "left" | null => {
    if (!rulersOn() || !straight(cx)) return null;
    const band = RULER * cx.dpr();
    if (p[1] < band && p[0] >= band) return "top";
    if (p[0] < band && p[1] >= band) return "left";
    return p[0] < band && p[1] < band ? "top" : null;
  };
  const place = (event: PointerEvent, cx: ViewportContext) => {
    if (!drag) return;
    const [x, y] = cx.toDoc(event.clientX, event.clientY);
    const raw = drag.vertical ? x : y;
    // whole pixels unless alt is held, like photoshop's guides
    drag.position = event.altKey ? Math.round(raw * 100) / 100 : Math.round(raw);
  };
  return {
    ...tool,
    down: (event, cx) => {
      if (event.button === 0 && s) {
        const p = local(event, cx);
        const ruler = inRuler(p, cx);
        if (ruler) {
          drag = { id: null, vertical: ruler === "left", position: 0 };
          place(event, cx);
          return true;
        }
        if (env.canDragGuides && a.guides && !a.lockGuides) {
          const hit = nearGuide(s.guides, cx, p);
          if (hit) {
            drag = { id: hit.id, vertical: hit.vertical, position: hit.position };
            return true;
          }
        }
      }
      return tool.down?.(event, cx) ?? false;
    },
    move: (event, cx) => {
      if (drag) {
        place(event, cx);
        cx.redraw();
        return;
      }
      tool.move?.(event, cx);
    },
    up: (event, cx) => {
      if (drag) {
        const done = drag;
        drag = null;
        const back = inRuler(local(event, cx), cx) !== null;
        const run = (kind: string, params: unknown) => {
          ipc.imageCommand(env.doc, kind, params).catch(env.fail);
        };
        if (done.id === null) {
          if (!back) run("image.guide.add", { vertical: done.vertical, position: done.position });
        } else if (back) {
          run("image.guide.move", { id: done.id, position: null });
        } else {
          run("image.guide.move", { id: done.id, position: done.position });
        }
        cx.redraw();
        return;
      }
      tool.up?.(event, cx);
      env.lines.current = [];
    },
    cancel: (cx) => {
      drag = null;
      env.lines.current = [];
      tool.cancel?.(cx);
    },
    draw: (g, cx, time) => {
      if (s && a.grid) drawGrid(g, cx, s, a);
      if (s && a.guides) {
        const shown = drag?.id != null ? s.guides.filter((x) => x.id !== drag?.id) : s.guides;
        drawGuides(g, cx, shown, GUIDE);
      }
      const more = tool.draw?.(g, cx, time) ?? false;
      if (env.lines.current.length > 0) {
        drawGuides(
          g,
          cx,
          env.lines.current.map((l) => ({ vertical: l.vertical, position: l.at })),
          SMART,
        );
      }
      if (drag) {
        drawGuides(g, cx, [drag], GUIDE);
        const d = cx.dpr();
        const [vx, vy] = drag.vertical ? cx.toView(drag.position, 0) : cx.toView(0, drag.position);
        g.font = `${String(11 * d)}px Inter Variable, sans-serif`;
        g.fillStyle = GUIDE;
        const label = `${drag.vertical ? "X" : "Y"} ${String(drag.position)} px`;
        if (drag.vertical) g.fillText(label, vx + 4 * d, (RULER + 14) * d);
        else g.fillText(label, (RULER + 4) * d, vy - 4 * d);
      }
      if (rulersOn() && straight(cx)) drawRulers(g, cx);
      return more;
    },
  };
}
