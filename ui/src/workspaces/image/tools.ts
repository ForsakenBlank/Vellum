import type { BrushSettings } from "../../bindings/BrushSettings";
import type { ColorStop } from "../../bindings/ColorStop";
import type { ImageState } from "../../bindings/ImageState";
import type { Shape } from "../../bindings/Shape";
import type { StrokePoint } from "../../bindings/StrokePoint";
import type { ViewportApi, ViewportContext, ViewportTool } from "../../components/viewport/tool";
import { ipc } from "../../lib/ipc";
import type { Snapper } from "./aids";
import { combineFor, findLayer, marqueeBox, snapAngle, snapBox } from "./geometry";
import type { ToolState } from "./store";

// the image editor's tools. each is a plain object the viewport calls with pointer events;
// what a tool shows while it works is drawn on the viewport's overlay, and what it does is
// one document command when it finishes, so every tool's result undoes in one step.

export interface ToolEnv {
  doc: string;
  state: ImageState | null;
  tools: ToolState;
  /** the viewport's zoom commands, once it is ready */
  view: ViewportApi | null;
  /** says what went wrong in a toast */
  fail: (err: unknown) => void;
  pickColor: (rgb: [number, number, number], background: boolean) => void;
  /** lines drags up with guides, the grid, the canvas and other layers */
  snap: Snapper;
}

type Pt = [number, number];

const ACCENT = "#ffb547";

/** runs `task` with the newest value, never more than one at a time */
export function latest<T>(task: (value: T) => Promise<unknown>) {
  let busy = false;
  let next: { value: T } | null = null;
  const pump = () => {
    if (busy || !next) return;
    const { value } = next;
    next = null;
    busy = true;
    void task(value).finally(() => {
      busy = false;
      pump();
    });
  };
  return (value: T) => {
    next = { value };
    pump();
  };
}

function strokePath(
  g: CanvasRenderingContext2D,
  cx: ViewportContext,
  pts: readonly Pt[],
  close: boolean,
) {
  g.beginPath();
  pts.forEach((p, i) => {
    const [x, y] = cx.toView(p[0], p[1]);
    if (i === 0) g.moveTo(x, y);
    else g.lineTo(x, y);
  });
  if (close) g.closePath();
}

/** a line that shows on any picture: dark under, light dashes over */
function antsLine(g: CanvasRenderingContext2D, cx: ViewportContext, offset: number) {
  const d = cx.dpr();
  g.lineWidth = d;
  g.setLineDash([]);
  g.strokeStyle = "rgba(255,255,255,0.95)";
  g.stroke();
  g.setLineDash([4 * d, 4 * d]);
  g.lineDashOffset = -offset * d;
  g.strokeStyle = "rgba(0,0,0,0.95)";
  g.stroke();
  g.setLineDash([]);
}

function circle(g: CanvasRenderingContext2D, cx: ViewportContext, at: Pt, radius: number) {
  const [x, y] = cx.toView(at[0], at[1]);
  const r = radius * cx.camera().zoom;
  const d = cx.dpr();
  if (r < 3 * d) {
    // too small to read as a circle, show a cross instead
    g.beginPath();
    g.moveTo(x - 5 * d, y);
    g.lineTo(x + 5 * d, y);
    g.moveTo(x, y - 5 * d);
    g.lineTo(x, y + 5 * d);
  } else {
    g.beginPath();
    g.arc(x, y, r, 0, Math.PI * 2);
  }
  g.lineWidth = d;
  g.strokeStyle = "rgba(0,0,0,0.7)";
  g.stroke();
  g.lineWidth = d * 0.6;
  g.strokeStyle = "rgba(255,255,255,0.9)";
  g.stroke();
}

function run(env: ToolEnv, kind: string, params: unknown) {
  ipc.imageCommand(env.doc, kind, params).catch(env.fail);
}

/** marching ants for the selection outline, in document pixels */
export function drawAnts(
  g: CanvasRenderingContext2D,
  cx: ViewportContext,
  loops: readonly (readonly Pt[])[],
  time: number,
): boolean {
  if (loops.length === 0) return false;
  const offset = (time / 60) % 8;
  for (const loop of loops) {
    strokePath(g, cx, loop, true);
    antsLine(g, cx, offset);
  }
  return true;
}

function point(event: PointerEvent, cx: ViewportContext): StrokePoint {
  const [x, y] = cx.toDoc(event.clientX, event.clientY);
  // a mouse reports half pressure while pressed; it paints at full strength like photoshop
  const pen = event.pointerType === "pen" || event.pointerType === "touch";
  return {
    x,
    y,
    pressure: pen ? (event.pressure > 0 ? event.pressure : 0.5) : 1,
    tiltX: event.tiltX,
    tiltY: event.tiltY,
  };
}

function brushTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  const erasing = t.tool === "eraser";
  const base: BrushSettings = erasing ? t.eraser : t.brush;
  let hover: Pt | null = null;
  let last: Pt | null = null;
  let queue: StrokePoint[] = [];
  let started: Promise<unknown> | null = null;
  let sending: Promise<unknown> = Promise.resolve();
  let down = false;

  const flush = () => {
    if (!started || queue.length === 0) return;
    const batch = queue;
    queue = [];
    sending = Promise.all([started, sending])
      .then(() => ipc.imageStroke(env.doc, batch))
      .catch(env.fail);
  };

  return {
    cursor: "none",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      const p = point(event, cx);
      const settings: BrushSettings = {
        ...base,
        tool: erasing ? "eraser" : t.brushKind === "pencil" ? "pencil" : "brush",
        color: t.foreground,
        background: t.background,
        seed: Math.floor(Math.random() * 1e9),
      };
      down = true;
      started = ipc.imageStrokeBegin(env.doc, settings).catch((err: unknown) => {
        down = false;
        env.fail(err);
        throw err;
      });
      // shift click draws a straight line from where the last stroke ended
      queue = event.shiftKey && last ? [{ ...p, x: last[0], y: last[1] }, p] : [p];
      flush();
      return true;
    },
    move: (event, cx) => {
      hover = cx.toDoc(event.clientX, event.clientY);
      if (!down) return;
      const events =
        typeof event.getCoalescedEvents === "function" ? event.getCoalescedEvents() : [];
      for (const e of events.length > 0 ? events : [event]) queue.push(point(e, cx));
      flush();
    },
    up: (event, cx) => {
      if (!down) return;
      down = false;
      queue.push(point(event, cx));
      flush();
      last = cx.toDoc(event.clientX, event.clientY);
      const begun = started;
      started = null;
      void Promise.all([begun, sending])
        .then(() => ipc.imageStrokeEnd(env.doc))
        .catch(env.fail);
    },
    cancel: () => {
      if (!down) return;
      down = false;
      started = null;
      queue = [];
      void sending.then(() => ipc.imageStrokeCancel(env.doc));
    },
    draw: (g, cx) => {
      if (hover) circle(g, cx, hover, base.size / 2);
      return false;
    },
  };
}

function marqueeTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  let start: Pt | null = null;
  let end: Pt | null = null;
  let mode = t.selectMode;
  let keys = { shift: false, alt: false };
  const box = () =>
    start && end
      ? marqueeBox(
          start,
          end,
          keys.shift && mode === t.selectMode,
          keys.alt && mode === t.selectMode,
        )
      : null;
  return {
    cursor: "crosshair",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      start = env.snap.point(cx.toDoc(event.clientX, event.clientY), cx, event);
      end = start;
      // modifiers at the start pick add or subtract, during the drag they shape the box
      mode = combineFor(t.selectMode, event.shiftKey, event.altKey);
      keys = { shift: false, alt: false };
      return true;
    },
    move: (event, cx) => {
      if (!start) return;
      end = env.snap.point(cx.toDoc(event.clientX, event.clientY), cx, event);
      keys = { shift: event.shiftKey, alt: event.altKey };
    },
    up: () => {
      const b = box();
      start = null;
      if (!b) return;
      const s = snapBox(b);
      if (s.w < 1 || s.h < 1) {
        // a click without a drag deselects, like photoshop
        if (mode === "replace" && env.state?.selection) run(env, "image.select.op", { op: "none" });
        return;
      }
      const shape: Shape =
        t.marquee === "ellipse" ? { type: "ellipse", ...s } : { type: "rect", ...s };
      run(env, "image.select.shape", { shape, mode, antiAlias: t.antiAlias, feather: t.feather });
    },
    cancel: () => {
      start = null;
    },
    draw: (g, cx) => {
      const b = box();
      if (!b) return false;
      const s = snapBox(b);
      if (t.marquee === "ellipse") {
        const [x0, y0] = cx.toView(s.x, s.y);
        const [x1, y1] = cx.toView(s.x + s.w, s.y + s.h);
        g.beginPath();
        g.ellipse(
          (x0 + x1) / 2,
          (y0 + y1) / 2,
          Math.abs(x1 - x0) / 2,
          Math.abs(y1 - y0) / 2,
          (cx.camera().rotation * Math.PI) / 180,
          0,
          Math.PI * 2,
        );
      } else {
        strokePath(
          g,
          cx,
          [
            [s.x, s.y],
            [s.x + s.w, s.y],
            [s.x + s.w, s.y + s.h],
            [s.x, s.y + s.h],
          ],
          true,
        );
      }
      antsLine(g, cx, 0);
      return false;
    },
  };
}

function lassoTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  let pts: Pt[] = [];
  let live: Pt[] = [];
  let hover: Pt | null = null;
  let mode = t.selectMode;
  let drawing = false;
  const magnetic = t.lasso === "magnetic";
  const polygon = t.lasso === "polygon" || magnetic;
  const wire = latest(async ([from, to]: [Pt, Pt]) => {
    const path = await ipc.imageMagnetic(env.doc, from, to, t.magneticWidth).catch(() => null);
    if (!path || !drawing) return;
    live = path.map((p) => [p[0] + 0.5, p[1] + 0.5]);
    // drop an anchor once the live path is long enough, so it does not wander
    if (live.length > 40) {
      pts.push(...live.slice(1, -10));
      live = live.slice(-10);
    }
  });
  const finish = () => {
    const all = [...pts, ...live];
    drawing = false;
    pts = [];
    live = [];
    if (magnetic) void ipc.imageMagneticEnd(env.doc);
    if (all.length < 3) return;
    run(env, "image.select.shape", {
      shape: { type: "polygon", points: all },
      mode,
      antiAlias: t.antiAlias,
      feather: t.feather,
    });
  };
  const cancel = () => {
    drawing = false;
    pts = [];
    live = [];
    if (magnetic) void ipc.imageMagneticEnd(env.doc);
  };
  const nearStart = (p: Pt, cx: ViewportContext) => {
    const first = pts[0];
    if (!first || pts.length < 3) return false;
    const [ax, ay] = cx.toView(first[0], first[1]);
    const [bx, by] = cx.toView(p[0], p[1]);
    return Math.hypot(ax - bx, ay - by) < 8 * cx.dpr();
  };
  return {
    cursor: "crosshair",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      const p = cx.toDoc(event.clientX, event.clientY);
      if (!drawing) {
        drawing = true;
        mode = combineFor(t.selectMode, event.shiftKey, event.altKey);
        pts = [p];
        live = [];
        if (magnetic) {
          ipc.imageMagneticBegin(env.doc, t.sampleAll, t.magneticContrast).catch((err: unknown) => {
            cancel();
            env.fail(err);
          });
        }
        return true;
      }
      if (polygon) {
        if (nearStart(p, cx)) {
          finish();
          return true;
        }
        if (magnetic) pts.push(...live.slice(1));
        else pts.push(p);
        live = [];
      }
      return true;
    },
    move: (event, cx) => {
      hover = cx.toDoc(event.clientX, event.clientY);
      if (!drawing) return;
      if (!polygon) {
        if (event.buttons & 1) pts.push(hover);
        return;
      }
      const from = pts[pts.length - 1];
      if (magnetic && from) {
        wire([
          [Math.floor(from[0]), Math.floor(from[1])],
          [Math.floor(hover[0]), Math.floor(hover[1])],
        ]);
      }
    },
    up: () => {
      if (!polygon && drawing) finish();
    },
    doubleClick: () => {
      if (polygon && drawing) {
        finish();
        return true;
      }
      return false;
    },
    key: (event) => {
      if (!drawing) return false;
      if (event.key === "Escape") {
        cancel();
        return true;
      }
      if (event.key === "Enter") {
        finish();
        return true;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        if (pts.length > 1) pts.pop();
        live = [];
        return true;
      }
      return false;
    },
    cancel: () => {
      if (!polygon) cancel();
    },
    draw: (g, cx) => {
      if (!drawing) return false;
      const shown = [...pts, ...live];
      if (polygon && !magnetic && hover) shown.push(hover);
      strokePath(g, cx, shown, false);
      antsLine(g, cx, 0);
      if (polygon) {
        for (const p of [pts[0]]) {
          if (!p) continue;
          const [x, y] = cx.toView(p[0], p[1]);
          g.fillStyle = ACCENT;
          g.fillRect(x - 3 * cx.dpr(), y - 3 * cx.dpr(), 6 * cx.dpr(), 6 * cx.dpr());
        }
      }
      return false;
    },
  };
}

function wandTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  if (t.wand === "wand") {
    return {
      cursor: "crosshair",
      down: (event, cx) => {
        if (event.button !== 0) return false;
        const [x, y] = cx.toDoc(event.clientX, event.clientY);
        run(env, "image.select.wand", {
          x: Math.floor(x),
          y: Math.floor(y),
          tolerance: t.tolerance,
          contiguous: t.contiguous,
          sampleAll: t.sampleAll,
          antiAlias: t.antiAlias,
          mode: combineFor(t.selectMode, event.shiftKey, event.altKey),
        });
        return true;
      },
    };
  }
  let pts: Pt[] = [];
  let hover: Pt | null = null;
  let subtract = false;
  return {
    cursor: "none",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      subtract = event.altKey || t.selectMode === "subtract";
      pts = [cx.toDoc(event.clientX, event.clientY)];
      return true;
    },
    move: (event, cx) => {
      hover = cx.toDoc(event.clientX, event.clientY);
      if (pts.length > 0 && event.buttons & 1) pts.push(hover);
    },
    up: () => {
      if (pts.length === 0) return;
      run(env, "image.select.quick", {
        points: pts,
        radius: t.quickSize / 2,
        sampleAll: t.sampleAll,
        subtract,
      });
      pts = [];
    },
    cancel: () => {
      pts = [];
    },
    draw: (g, cx) => {
      if (pts.length > 1) {
        strokePath(g, cx, pts, false);
        g.lineWidth = t.quickSize * cx.camera().zoom;
        g.lineCap = "round";
        g.lineJoin = "round";
        g.strokeStyle = subtract ? "rgba(255,90,90,0.25)" : "rgba(255,181,71,0.25)";
        g.stroke();
        g.lineCap = "butt";
      }
      if (hover) circle(g, cx, hover, t.quickSize / 2);
      return false;
    },
  };
}

function moveTool(env: ToolEnv): ViewportTool {
  let start: Pt | null = null;
  let delta: Pt = [0, 0];
  const preview = latest((d: Pt) =>
    ipc
      .imagePreview(env.doc, {
        offset: [layerIds(env), d[0], d[1]],
        showMask: null,
        transform: null,
      })
      .catch(() => undefined),
  );
  const bounds = () => {
    const s = env.state;
    const id = s?.active;
    const b = s && id !== null && id !== undefined ? findLayer(s.layers, id)?.bounds : undefined;
    return b && b.w > 0 ? b : null;
  };
  const commit = (dx: number, dy: number) => {
    if (dx === 0 && dy === 0) return;
    run(env, "image.layer.offset", { ids: layerIds(env), dx, dy });
  };
  return {
    cursor: "move",
    down: (event, cx) => {
      if (event.button !== 0 || layerIds(env).length === 0) return false;
      start = cx.toDoc(event.clientX, event.clientY);
      delta = [0, 0];
      return true;
    },
    move: (event, cx) => {
      if (!start) return;
      const [x, y] = cx.toDoc(event.clientX, event.clientY);
      let dx = Math.round(x - start[0]);
      let dy = Math.round(y - start[1]);
      // shift keeps the move straight across or up and down
      if (event.shiftKey) {
        if (Math.abs(dx) > Math.abs(dy)) dy = 0;
        else dx = 0;
      }
      const b = bounds();
      if (b) {
        const [sx, sy] = env.snap.move(b, dx, dy, cx, event);
        dx = Math.round(sx);
        dy = Math.round(sy);
      }
      delta = [dx, dy];
      preview(delta);
    },
    up: () => {
      if (!start) return;
      start = null;
      env.snap.clear();
      const [dx, dy] = delta;
      void ipc
        .imagePreview(env.doc, { offset: null, showMask: null, transform: null })
        .then(() => {
          commit(dx, dy);
        })
        .catch(env.fail);
    },
    cancel: () => {
      start = null;
      env.snap.clear();
      void ipc.imagePreview(env.doc, { offset: null, showMask: null, transform: null });
    },
    key: (event) => {
      const step = event.shiftKey ? 10 : 1;
      const moves: Record<string, Pt> = {
        ArrowLeft: [-step, 0],
        ArrowRight: [step, 0],
        ArrowUp: [0, -step],
        ArrowDown: [0, step],
      };
      const m = moves[event.key];
      if (!m) return false;
      commit(m[0], m[1]);
      return true;
    },
  };
}

function layerIds(env: ToolEnv): number[] {
  const id = env.state?.active;
  return id === null || id === undefined ? [] : [id];
}

function gradientTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  if (t.fill === "bucket") {
    return {
      cursor: "crosshair",
      down: (event, cx) => {
        if (event.button !== 0) return false;
        const [x, y] = cx.toDoc(event.clientX, event.clientY);
        run(env, "image.paint.bucket", {
          x: Math.floor(x),
          y: Math.floor(y),
          settings: { ...t.bucket, color: t.foreground },
        });
        return true;
      },
    };
  }
  let start: Pt | null = null;
  let end: Pt | null = null;
  return {
    cursor: "crosshair",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      start = cx.toDoc(event.clientX, event.clientY);
      end = start;
      return true;
    },
    move: (event, cx) => {
      if (!start) return;
      const p = cx.toDoc(event.clientX, event.clientY);
      end = event.shiftKey ? snapAngle(start, p) : p;
    },
    up: () => {
      if (!start || !end) return;
      const [from, to] = [start, end];
      start = null;
      if (Math.hypot(to[0] - from[0], to[1] - from[1]) < 1) return;
      const fg = t.foreground;
      const stops: ColorStop[] = [
        { at: 0, color: [fg[0], fg[1], fg[2], 1] },
        t.gradientToClear
          ? { at: 1, color: [fg[0], fg[1], fg[2], 0] }
          : { at: 1, color: [t.background[0], t.background[1], t.background[2], 1] },
      ];
      run(env, "image.paint.gradient", {
        from,
        to,
        settings: {
          kind: t.gradientKind,
          stops,
          reverse: t.gradientReverse,
          dither: t.gradientDither,
          opacity: t.gradientOpacity,
          mode: "normal",
        },
      });
    },
    cancel: () => {
      start = null;
    },
    draw: (g, cx) => {
      if (!start || !end) return false;
      strokePath(g, cx, [start, end], false);
      antsLine(g, cx, 0);
      for (const p of [start, end]) {
        const [x, y] = cx.toView(p[0], p[1]);
        g.fillStyle = ACCENT;
        g.beginPath();
        g.arc(x, y, 3.5 * cx.dpr(), 0, Math.PI * 2);
        g.fill();
      }
      return false;
    },
  };
}

function eyedropperTool(env: ToolEnv): ViewportTool {
  const t = env.tools;
  let down = false;
  const pick = latest(async ([x, y, bg]: [number, number, boolean]) => {
    const c = await ipc
      .imagePickColor(env.doc, Math.floor(x), Math.floor(y), t.sampleSize, t.sampleLayers === "all")
      .catch(() => null);
    if (c && c[3] > 0) env.pickColor([c[0], c[1], c[2]], bg);
  });
  return {
    cursor: "crosshair",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      down = true;
      const [x, y] = cx.toDoc(event.clientX, event.clientY);
      pick([x, y, event.altKey]);
      return true;
    },
    move: (event, cx) => {
      if (!down) return;
      const [x, y] = cx.toDoc(event.clientX, event.clientY);
      pick([x, y, event.altKey]);
    },
    up: () => {
      down = false;
    },
  };
}

function zoomTool(env: ToolEnv): ViewportTool {
  return {
    cursor: "zoom-in",
    down: (event, cx) => {
      if (event.button !== 0) return false;
      const [x, y] = cx.toDoc(event.clientX, event.clientY);
      env.view?.zoomAround(x, y, event.altKey ? 0.5 : 2);
      return true;
    },
  };
}

/** the tool for the current state, with the selection's ants drawn under it */
export function makeTool(env: ToolEnv, ants: readonly (readonly Pt[])[]): ViewportTool {
  const t = env.tools;
  const tool: ViewportTool =
    t.tool === "brush" || t.tool === "eraser"
      ? brushTool(env)
      : t.tool === "marquee"
        ? marqueeTool(env)
        : t.tool === "lasso"
          ? lassoTool(env)
          : t.tool === "wand"
            ? wandTool(env)
            : t.tool === "move"
              ? moveTool(env)
              : t.tool === "gradient"
                ? gradientTool(env)
                : t.tool === "eyedropper"
                  ? eyedropperTool(env)
                  : t.tool === "zoom"
                    ? zoomTool(env)
                    : { cursor: "grab" };
  const own = tool.draw;
  return {
    ...tool,
    draw: (g, cx, time) => {
      const animating = drawAnts(g, cx, ants, time);
      const more = own?.(g, cx, time) ?? false;
      return animating || more;
    },
  };
}
