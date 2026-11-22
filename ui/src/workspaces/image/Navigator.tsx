import { useEffect, useRef, useState } from "react";

import type { ViewportApi } from "../../components/viewport/tool";
import { ipc } from "../../lib/ipc";
import { MAX_ZOOM, MIN_ZOOM, toDocument, zoomLabel } from "../../lib/viewport/camera";
import { type Thumb, parseThumbnails, thumbKey } from "./geometry";
import { useViewCamera } from "./viewCamera";

/** the navigator's picture is this wide at most, in css pixels */
const WIDE = 240;
const TALL = 150;

/**
 * a small picture of the whole document with the part the view shows outlined. drag in
 * it to move the view, and use the slider to zoom.
 */
export function Navigator({
  doc,
  width,
  height,
  revision,
  view,
}: {
  doc: string;
  width: number;
  height: number;
  /** changes whenever the picture does */
  revision: string;
  view: ViewportApi | null;
}) {
  const camera = useViewCamera((s) => s.views[doc] ?? null);
  const [thumb, setThumb] = useState<Thumb | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);

  // the picture, a moment after the last change
  useEffect(() => {
    let stale = false;
    const timer = window.setTimeout(() => {
      ipc
        .imageThumbnails(doc, [[0, false]], WIDE * 2)
        .then((buf) => {
          if (!stale) setThumb(parseThumbnails(buf).get(thumbKey(0, false)) ?? null);
        })
        .catch(() => undefined);
    }, 400);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [doc, revision]);

  const fitScale = Math.min(WIDE / width, TALL / height);
  const w = Math.max(1, Math.round(width * fitScale));
  const h = Math.max(1, Math.round(height * fitScale));

  useEffect(() => {
    const el = canvas.current;
    const g = el?.getContext("2d");
    if (!el || !g) return;
    const dpr = window.devicePixelRatio || 1;
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);
    if (thumb) {
      const img = new ImageData(thumb.pixels, thumb.width, thumb.height);
      const tmp = document.createElement("canvas");
      tmp.width = thumb.width;
      tmp.height = thumb.height;
      tmp.getContext("2d")?.putImageData(img, 0, 0);
      g.drawImage(tmp, 0, 0, w, h);
    }
    if (camera) {
      const { camera: cam, size } = camera;
      const corners = [
        toDocument(cam, size, 0, 0),
        toDocument(cam, size, size.width, 0),
        toDocument(cam, size, size.width, size.height),
        toDocument(cam, size, 0, size.height),
      ];
      g.beginPath();
      corners.forEach(([x, y], i) => {
        const px = x * fitScale;
        const py = y * fitScale;
        if (i === 0) g.moveTo(px, py);
        else g.lineTo(px, py);
      });
      g.closePath();
      g.lineWidth = 2;
      g.strokeStyle = "rgba(0,0,0,0.6)";
      g.stroke();
      g.lineWidth = 1;
      g.strokeStyle = "#ffb547";
      g.stroke();
    }
  }, [thumb, camera, w, h, fitScale]);

  const go = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    view?.centerOn((e.clientX - r.left) / fitScale, (e.clientY - r.top) / fitScale);
  };

  const zoom = camera?.camera.zoom ?? 1;
  // the slider moves along powers of two, so each notch feels the same
  const toSlider = (z: number) => Math.log2(z);
  return (
    <section aria-label="Navigator" className="flex flex-col gap-1.5 border-b border-line p-2.5">
      <h3 className="text-xs font-medium text-fg-muted">Navigator</h3>
      <div className="flex justify-center rounded-md bg-canvas p-1">
        <canvas
          ref={canvas}
          aria-label="Drag to move the view"
          style={{ width: w, height: h }}
          className="cursor-move touch-none"
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId);
            go(e);
          }}
          onPointerMove={(e) => {
            if (e.buttons & 1) go(e);
          }}
        />
      </div>
      <label className="flex items-center gap-2 text-2xs text-fg-subtle">
        <span className="w-12 tabular-nums">{zoomLabel(zoom)}</span>
        <input
          type="range"
          aria-label="Zoom"
          min={toSlider(MIN_ZOOM)}
          max={toSlider(MAX_ZOOM)}
          step={0.05}
          value={toSlider(zoom)}
          onChange={(e) => {
            view?.setZoom(2 ** Number(e.target.value));
          }}
          className="flex-1 accent-(--accent)"
        />
      </label>
    </section>
  );
}
