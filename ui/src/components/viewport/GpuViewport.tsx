import {
  FlipHorizontal2,
  Grid3x3,
  Maximize,
  RotateCcw,
  RotateCw,
  Scan,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import type { ViewInfo } from "../../bindings/ViewInfo";
import { onRustEvent } from "../../lib/events";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { isMac } from "../../lib/keys";
import {
  type Camera,
  type Size,
  fit,
  flip,
  panBy,
  rotateAround,
  stepZoom,
  zoomAt,
  zoomLabel,
  zoomTo,
} from "../../lib/viewport/camera";
import { parseFrame, tileKey } from "../../lib/viewport/frame";
import { PIXEL_GRID_ZOOM, type Palette, TileRenderer } from "../../lib/viewport/renderer";
import { TileCache } from "../../lib/viewport/tileCache";
import { useSettings } from "../../state/settings";
import { IconButton } from "../Button";
import { Tooltip } from "../Tooltip";
import { type ViewportApi, type ViewportContext, type ViewportTool, makeContext } from "./tool";

/** textures the window may keep, about 1000 tiles */
const TILE_BUDGET = 256 * 1024 * 1024;
/** how far one wheel notch zooms */
const WHEEL_ZOOM = 0.0015;
const ROTATE_STEP = 15;
/** degrees per pixel of wheel travel with shift held, about 5 degrees a notch */
const WHEEL_TURN = 0.06;

interface Props {
  /** the document id, which names the viewport in rust */
  document: string;
  title: string;
  /** what pointer presses do besides panning, with an overlay drawn over the picture */
  tool?: ViewportTool | undefined;
  /** called once the view can take zoom commands */
  onReady?: ((api: ViewportApi) => void) | undefined;
  /** called when the zoom changes, eg so marching ants can ask for a matching outline */
  onZoom?: ((zoom: number) => void) | undefined;
}

interface Hud {
  zoom: number;
  rotation: number;
  flipped: boolean;
}

/** a css colour token as rgb in [0, 1] */
function token(name: string, fallback: [number, number, number]): [number, number, number] {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value);
  if (!m) return fallback;
  const [, r = "0", g = "0", b = "0"] = m;
  return [parseInt(r, 16) / 255, parseInt(g, 16) / 255, parseInt(b, 16) / 255];
}

function readPalette(): Palette {
  const fg = token("--fg-1", [0.9, 0.9, 0.9]);
  return {
    outside: token("--surface-0", [0.05, 0.05, 0.06]),
    checkerA: token("--surface-2", [0.09, 0.09, 0.11]),
    checkerB: token("--surface-3", [0.12, 0.13, 0.15]),
    grid: [fg[0], fg[1], fg[2], 0.22],
  };
}

const mod = isMac ? "Cmd" : "Ctrl";

/**
 * a picture rendered by the rust engine, drawn with webgl. moving the view never waits
 * for rust: the window places the tiles it holds at the display's refresh rate and asks
 * for missing ones in the background, one request at a time.
 */
export function GpuViewport({ document: docId, title, tool, onReady, onZoom }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlay = useRef<HTMLCanvasElement>(null);
  const toolRef = useRef<ViewportTool | undefined>(tool);
  const onZoomRef = useRef(onZoom);
  useEffect(() => {
    onZoomRef.current = onZoom;
  }, [onZoom]);
  const overlayDirty = useRef(true);
  const [reopen, setReopen] = useState(0);
  const [info, setInfo] = useState<ViewInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hud, setHud] = useState<Hud>({ zoom: 1, rotation: 0, flipped: false });
  const [grid, setGrid] = useState(true);
  const theme = useSettings((s) => s.settings?.appearance.theme);

  // everything the frame loop touches lives in refs, so moving the view never re-renders
  const cam = useRef<Camera>({ centerX: 0, centerY: 0, zoom: 1, rotation: 0, flipX: false });
  const size = useRef<Size>({ width: 1, height: 1 });
  const doc = useRef<ViewInfo | null>(null);
  const renderer = useRef<TileRenderer | null>(null);
  const cache = useRef<TileCache<WebGLTexture> | null>(null);
  const palette = useRef<Palette | null>(null);
  const gridOn = useRef(grid);
  const loop = useRef({
    dirty: true,
    wanted: true,
    inflight: false,
    pending: 0,
    reset: true,
    evicted: [] as { level: number; col: number; row: number }[],
    frames: 0,
    drawMs: 0,
  });

  const changed = useCallback(() => {
    loop.current.dirty = true;
    loop.current.wanted = true;
    overlayDirty.current = true;
  }, []);

  // made once the component is mounted, since it reads the refs above when called
  const context = useRef<ViewportContext | null>(null);
  useEffect(() => {
    context.current = makeContext(cam, size, canvas, overlayDirty);
  }, []);

  useEffect(() => {
    toolRef.current = tool;
    overlayDirty.current = true;
  }, [tool]);

  // the picture changed in rust: ask for new tiles, or start over if the canvas resized
  useEffect(() => {
    if (!inDesktopApp) return;
    let stale = false;
    const unlisten = onRustEvent("image:changed", (event) => {
      if (stale || event.document !== docId) return;
      const d = doc.current;
      if (event.width > 0 && d && (d.width !== event.width || d.height !== event.height)) {
        setReopen((n) => n + 1);
        return;
      }
      if (event.width === 0) loop.current.reset = true;
      changed();
    });
    return () => {
      stale = true;
      void unlisten.then((f) => {
        f();
      });
    };
  }, [docId, changed]);

  const setCamera = useCallback(
    (next: Camera) => {
      cam.current = next;
      changed();
    },
    [changed],
  );

  // open the picture in rust
  useEffect(() => {
    let stale = false;
    ipc
      .openView(docId)
      .then((opened) => {
        if (stale) return;
        const resized = doc.current !== null;
        doc.current = opened;
        setInfo(opened);
        if (!resized) cam.current = fit(opened, size.current);
        cache.current?.clear();
        loop.current.reset = true;
        changed();
      })
      .catch((err: unknown) => {
        if (!stale) setError(String(err));
      });
    // rust keeps the view while the document is open, so coming back to this tab is
    // instant; closing the document closes the view
    return () => {
      stale = true;
    };
  }, [docId, changed, reopen]);

  useEffect(() => {
    gridOn.current = grid;
    loop.current.dirty = true;
  }, [grid]);

  useEffect(() => {
    palette.current = readPalette();
    loop.current.dirty = true;
  }, [theme]);

  // webgl, the frame loop and the tile requests
  useEffect(() => {
    const el = canvas.current;
    const outer = box.current;
    if (!el || !outer) return;
    const start = () => {
      try {
        renderer.current = new TileRenderer(el);
        const r = renderer.current;
        cache.current = new TileCache(TILE_BUDGET, (t) => {
          r.release(t);
        });
        palette.current ??= readPalette();
      } catch (err) {
        setError(String(err));
      }
    };
    start();

    const lost = (event: Event) => {
      // a lost context comes back by itself; until then nothing can be drawn
      event.preventDefault();
      renderer.current = null;
      cache.current = null;
    };
    const restored = () => {
      start();
      loop.current.reset = true;
      changed();
    };
    el.addEventListener("webglcontextlost", lost);
    el.addEventListener("webglcontextrestored", restored);

    const resize = new ResizeObserver(() => {
      const dpr = window.devicePixelRatio || 1;
      const width = Math.max(1, Math.round(outer.clientWidth * dpr));
      const height = Math.max(1, Math.round(outer.clientHeight * dpr));
      const first = size.current.width <= 1;
      el.width = width;
      el.height = height;
      if (overlay.current) {
        overlay.current.width = width;
        overlay.current.height = height;
      }
      size.current = { width, height };
      if (first && doc.current) cam.current = fit(doc.current, size.current);
      changed();
    });
    resize.observe(outer);

    const request = () => {
      const s = loop.current;
      const picture = doc.current;
      if (s.inflight || !picture || !renderer.current) return;
      s.inflight = true;
      s.wanted = false;
      const c = cam.current;
      const view = {
        centerX: c.centerX,
        centerY: c.centerY,
        zoom: c.zoom,
        rotation: c.rotation,
        flipX: c.flipX,
        width: size.current.width,
        height: size.current.height,
      };
      const evicted = s.evicted.splice(0);
      const reset = s.reset;
      s.reset = false;
      ipc
        .viewFrame(docId, view, evicted, reset)
        .then((buffer) => {
          const r = renderer.current;
          const tiles = cache.current;
          if (!r || !tiles) return;
          const frame = parseFrame(buffer);
          for (const id of frame.dropped) tiles.drop(id);
          const keep = new Set(frame.tiles.map((t) => tileKey(t.id)));
          for (const t of frame.tiles) {
            const texture = r.upload(t.rect.w, t.rect.h, t.pixels);
            s.evicted.push(...tiles.put(t.id, t.rect, texture, t.pixels.byteLength, keep));
          }
          s.pending = frame.pending;
          s.dirty = true;
        })
        .catch((err: unknown) => {
          // the view closed, or rust could not render; say so once and stop asking
          setError(String(err));
          s.pending = 0;
        })
        .finally(() => {
          s.inflight = false;
        });
    };

    let raf = 0;
    let animating = false;
    let drewOverlay = false;
    let lastZoom = 0;
    const tick = (time: number) => {
      raf = requestAnimationFrame(tick);
      const s = loop.current;
      if (cam.current.zoom !== lastZoom) {
        lastZoom = cam.current.zoom;
        onZoomRef.current?.(lastZoom);
      }
      const over = overlay.current;
      const t = toolRef.current;
      const cx = context.current;
      // a view without a tool draws nothing over the picture, so it skips the overlay
      if (over && cx && (overlayDirty.current || animating) && (t?.draw || drewOverlay)) {
        overlayDirty.current = false;
        const g = over.getContext("2d");
        if (g) {
          g.setTransform(1, 0, 0, 1, 0, 0);
          g.clearRect(0, 0, over.width, over.height);
          animating = t?.draw?.(g, cx, time) ?? false;
          drewOverlay = Boolean(t?.draw);
        }
      }
      const r = renderer.current;
      const tiles = cache.current;
      const picture = doc.current;
      if (s.dirty && r && tiles && picture && palette.current) {
        s.dirty = false;
        const stats = r.draw(
          cam.current,
          size.current,
          picture,
          tiles.drawOrder(),
          palette.current,
          gridOn.current,
        );
        s.frames++;
        el.dataset.frames = String(s.frames);
        s.drawMs = s.drawMs * 0.9 + stats.cpuMs * 0.1;
        el.dataset.drawMs = s.drawMs.toFixed(2);
        el.dataset.tiles = String(tiles.size);
      }
      if ((s.wanted || s.pending > 0) && !s.inflight) request();
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      resize.disconnect();
      el.removeEventListener("webglcontextlost", lost);
      el.removeEventListener("webglcontextrestored", restored);
      cache.current?.clear();
      cache.current = null;
      renderer.current?.dispose();
      renderer.current = null;
    };
  }, [docId, changed]);

  // keep the readout in step with the camera, at most once per frame
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const c = cam.current;
      setHud((h) =>
        h.zoom === c.zoom && h.rotation === c.rotation && h.flipped === c.flipX
          ? h
          : { zoom: c.zoom, rotation: c.rotation, flipped: c.flipX },
      );
    };
    raf = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(raf);
    };
  }, []);

  const center = () => [size.current.width / 2, size.current.height / 2] as const;
  const zoomStep = (direction: 1 | -1) => {
    const [x, y] = center();
    setCamera(zoomTo(cam.current, size.current, x, y, stepZoom(cam.current.zoom, direction)));
  };
  const actualPixels = () => {
    const [x, y] = center();
    setCamera(zoomTo(cam.current, size.current, x, y, 1));
  };
  const fitView = () => {
    if (doc.current) setCamera(fit(doc.current, size.current));
  };
  const rotateBy = (deg: number) => {
    const [x, y] = center();
    setCamera(rotateAround(cam.current, size.current, x, y, deg));
  };
  const resetRotation = () => {
    rotateBy(-cam.current.rotation);
  };

  const ready = useRef(onReady);
  useEffect(() => {
    ready.current?.({
      fit: () => {
        if (doc.current) setCamera(fit(doc.current, size.current));
      },
      actualPixels: () => {
        setCamera(
          zoomTo(cam.current, size.current, size.current.width / 2, size.current.height / 2, 1),
        );
      },
      zoomStep: (direction) => {
        const [x, y] = [size.current.width / 2, size.current.height / 2];
        setCamera(zoomTo(cam.current, size.current, x, y, stepZoom(cam.current.zoom, direction)));
      },
      zoomAround: (x, y, factor) => {
        const cx = context.current;
        if (!cx) return;
        const [sx, sy] = cx.toView(x, y);
        setCamera(zoomAt(cam.current, size.current, sx, sy, factor));
      },
    });
  }, [setCamera]);

  // pointer and keyboard
  const drag = useRef<{ x: number; y: number; mode: "pan" | "rotate" } | null>(null);
  const held = useRef(new Set<string>());

  const toDevice = (event: { clientX: number; clientY: number }) => {
    const rect = canvas.current?.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    return [
      (event.clientX - (rect?.left ?? 0)) * dpr,
      (event.clientY - (rect?.top ?? 0)) * dpr,
    ] as const;
  };

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    // react's wheel listener is passive, and the page must not scroll under the picture
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1;
      const [x, y] = toDevice(event);
      if (event.shiftKey) {
        const turn = (event.deltaY || event.deltaX) * unit * WHEEL_TURN;
        setCamera(rotateAround(cam.current, size.current, x, y, turn));
        return;
      }
      const factor = Math.exp(-event.deltaY * unit * WHEEL_ZOOM * (event.ctrlKey ? 4 : 1));
      setCamera(zoomAt(cam.current, size.current, x, y, factor));
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", wheel);
    };
  }, [setCamera]);

  const onKeyDown = (event: React.KeyboardEvent) => {
    held.current.add(event.code);
    const cx = context.current;
    if (cx && toolRef.current?.key?.(event.nativeEvent, cx)) {
      event.preventDefault();
      overlayDirty.current = true;
      return;
    }
    const ctrl = isMac ? event.metaKey : event.ctrlKey;
    let handled = true;
    if (ctrl && event.code === "Digit0") fitView();
    else if (ctrl && event.code === "Digit1") actualPixels();
    else if (ctrl && (event.code === "Equal" || event.code === "NumpadAdd")) zoomStep(1);
    else if (ctrl && (event.code === "Minus" || event.code === "NumpadSubtract")) zoomStep(-1);
    else if (event.code === "Escape" && cam.current.rotation !== 0) resetRotation();
    else handled = false;
    // claiming the event keeps the app's own ctrl+0 and ctrl+= (interface zoom) out of it
    if (handled) event.preventDefault();
  };

  const onKeyUp = (event: React.KeyboardEvent) => {
    held.current.delete(event.code);
  };

  const toolDown = useRef(false);

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (event.button !== 0 && event.button !== 1 && event.button !== 2) return;
    event.currentTarget.focus();
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // the pointer is already gone (or was never a real one); the drag still works
    }
    const t = toolRef.current;
    const cx = context.current;
    const panning = event.button === 1 || held.current.has("Space") || held.current.has("KeyR");
    if (!panning && t?.down && cx) {
      if (t.down(event.nativeEvent, cx)) {
        toolDown.current = true;
        overlayDirty.current = true;
        return;
      }
      if (t.cursor !== "grab") return;
    }
    if (event.button === 2) return;
    const mode = held.current.has("KeyR") || (event.altKey && !t) ? "rotate" : "pan";
    drag.current = { x: event.clientX, y: event.clientY, mode };
  };

  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const t = toolRef.current;
    const cx = context.current;
    if (!drag.current && t?.move && cx) {
      t.move(event.nativeEvent, cx);
      overlayDirty.current = true;
    }
    const d = drag.current;
    if (!d) return;
    const dpr = window.devicePixelRatio || 1;
    if (d.mode === "pan") {
      setCamera(panBy(cam.current, (event.clientX - d.x) * dpr, (event.clientY - d.y) * dpr));
    } else {
      // turn by the angle the pointer swept around the middle of the view
      const rect = event.currentTarget.getBoundingClientRect();
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      const before = Math.atan2(d.y - cy, d.x - cx);
      const after = Math.atan2(event.clientY - cy, event.clientX - cx);
      const [x, y] = center();
      setCamera(rotateAround(cam.current, size.current, x, y, ((after - before) * 180) / Math.PI));
    }
    d.x = event.clientX;
    d.y = event.clientY;
  };

  const onPointerUp = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const cx = context.current;
    if (toolDown.current && cx) {
      toolDown.current = false;
      toolRef.current?.up?.(event.nativeEvent, cx);
      overlayDirty.current = true;
    }
    drag.current = null;
  };

  const onPointerCancel = () => {
    const cx = context.current;
    if (toolDown.current && cx) {
      toolDown.current = false;
      toolRef.current?.cancel?.(cx);
      overlayDirty.current = true;
    }
    drag.current = null;
  };

  const rotated = Math.abs(hud.rotation) > 1e-6;

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-line bg-canvas">
      <div ref={box} className="relative min-h-0 flex-1">
        <canvas
          ref={canvas}
          tabIndex={0}
          role="img"
          aria-label={`${title} at ${zoomLabel(hud.zoom)}`}
          data-testid="gpu-viewport"
          className={`absolute inset-0 size-full touch-none outline-none focus-visible:shadow-(--ring) ${tool ? "" : "cursor-grab active:cursor-grabbing"}`}
          style={tool ? { cursor: tool.cursor } : undefined}
          onContextMenu={(event) => {
            if (tool) event.preventDefault();
          }}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
          onBlur={() => {
            held.current.clear();
          }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerCancel}
          onPointerLeave={() => {
            overlayDirty.current = true;
          }}
          onDoubleClick={(event) => {
            const cx = context.current;
            if (cx && toolRef.current?.doubleClick?.(event.nativeEvent, cx)) return;
            if (tool) return;
            if (Math.abs(cam.current.zoom - 1) < 1e-6) fitView();
            else actualPixels();
          }}
        />
        <canvas
          ref={overlay}
          aria-hidden
          className="pointer-events-none absolute inset-0 size-full"
        />
        {error && (
          <div className="absolute inset-0 flex items-center justify-center p-6">
            <p className="max-w-96 rounded-lg bg-panel/90 px-4 py-3 text-center text-sm text-fg-muted shadow-2">
              {error}
            </p>
          </div>
        )}
      </div>
      <div
        role="toolbar"
        aria-label="View controls"
        className="flex shrink-0 items-center gap-0.5 border-t border-line bg-panel px-2 py-1"
      >
        <IconButton
          size="sm"
          label="Zoom out"
          shortcut={`${mod}+-`}
          icon={<ZoomOut className="size-3.5" />}
          onClick={() => {
            zoomStep(-1);
          }}
        />
        <Tooltip text="Fit the picture in the window" shortcut={`${mod}+0`}>
          <button
            type="button"
            onClick={fitView}
            className="h-6 min-w-14 rounded-md px-1.5 text-xs tabular-nums text-fg-muted hover:bg-elevated hover:text-fg"
          >
            {zoomLabel(hud.zoom)}
          </button>
        </Tooltip>
        <IconButton
          size="sm"
          label="Zoom in"
          shortcut={`${mod}+=`}
          icon={<ZoomIn className="size-3.5" />}
          onClick={() => {
            zoomStep(1);
          }}
        />
        <IconButton
          size="sm"
          label="Fit the picture in the window"
          shortcut={`${mod}+0`}
          icon={<Maximize className="size-3.5" />}
          onClick={fitView}
        />
        <IconButton
          size="sm"
          label="Actual pixels"
          shortcut={`${mod}+1`}
          icon={<Scan className="size-3.5" />}
          onClick={actualPixels}
        />
        <span className="mx-1 h-4 w-px bg-line" />
        <IconButton
          size="sm"
          label="Turn the view left. Hold R and drag to turn freely"
          icon={<RotateCcw className="size-3.5" />}
          onClick={() => {
            rotateBy(-ROTATE_STEP);
          }}
        />
        <IconButton
          size="sm"
          label="Turn the view right. Hold R and drag to turn freely"
          icon={<RotateCw className="size-3.5" />}
          onClick={() => {
            rotateBy(ROTATE_STEP);
          }}
        />
        {rotated && (
          <Tooltip text="Straighten the view" shortcut="Esc">
            <button
              type="button"
              onClick={resetRotation}
              className="h-6 rounded-md px-1.5 text-xs tabular-nums text-fg-muted hover:bg-elevated hover:text-fg"
            >
              {`${String(Math.round(hud.rotation))}°`}
            </button>
          </Tooltip>
        )}
        <IconButton
          size="sm"
          label="Mirror the view (the picture itself does not change)"
          active={hud.flipped}
          icon={<FlipHorizontal2 className="size-3.5" />}
          onClick={() => {
            setCamera(flip(cam.current));
          }}
        />
        <IconButton
          size="sm"
          label={`Pixel grid, shown from ${String(PIXEL_GRID_ZOOM * 100)}%`}
          active={grid}
          icon={<Grid3x3 className="size-3.5" />}
          onClick={() => {
            setGrid((g) => !g);
          }}
        />
        <span className="ml-auto truncate pl-2 text-2xs text-fg-subtle">
          {info
            ? `${String(info.width)} × ${String(info.height)} px${info.profile ? ` · ${info.profile}` : ""}`
            : error
              ? ""
              : "Opening…"}
        </span>
      </div>
    </div>
  );
}
