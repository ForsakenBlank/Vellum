import { type Camera, type Size, toDocument, toScreen } from "../../lib/viewport/camera";

/** what a tool can ask the viewport while it works */
export interface ViewportContext {
  /** the document pixel position under a point on the page */
  toDoc: (clientX: number, clientY: number) => [number, number];
  /** a document position as device pixels in the view */
  toView: (x: number, y: number) => [number, number];
  camera: () => Camera;
  size: () => Size;
  /** device pixels per css pixel */
  dpr: () => number;
  /** draws the overlay again on the next frame */
  redraw: () => void;
}

/**
 * a tool working in a viewport. the view pans with the space bar or the middle button
 * whatever the tool, so tools only see presses they can use.
 */
export interface ViewportTool {
  /** a css cursor */
  cursor: string;
  /** true if the tool took the press; otherwise the drag pans the view */
  down?: (event: PointerEvent, cx: ViewportContext) => boolean;
  /** pointer moves, pressed or not */
  move?: (event: PointerEvent, cx: ViewportContext) => void;
  up?: (event: PointerEvent, cx: ViewportContext) => void;
  /** the press was lost (the pointer left the window, escape) */
  cancel?: (cx: ViewportContext) => void;
  /** a key pressed while the view has focus; true if the tool used it */
  key?: (event: KeyboardEvent, cx: ViewportContext) => boolean;
  doubleClick?: (event: MouseEvent, cx: ViewportContext) => boolean;
  /** draws over the picture in device pixels. returns true to keep animating. */
  draw?: (g: CanvasRenderingContext2D, cx: ViewportContext, time: number) => boolean;
}

/** what the viewport offers to commands like "fit on screen" */
export interface ViewportApi {
  fit: () => void;
  actualPixels: () => void;
  zoomStep: (direction: 1 | -1) => void;
  /** zooms by a factor around a document point */
  zoomAround: (x: number, y: number, factor: number) => void;
}

interface Holder<T> {
  current: T;
}

/** a context reading the viewport's live camera, size and canvas */
export function makeContext(
  cam: Holder<Camera>,
  size: Holder<Size>,
  canvas: Holder<HTMLCanvasElement | null>,
  dirty: Holder<boolean>,
): ViewportContext {
  const dpr = () => window.devicePixelRatio || 1;
  return {
    camera: () => cam.current,
    size: () => size.current,
    dpr,
    redraw: () => {
      dirty.current = true;
    },
    toDoc: (clientX, clientY) => {
      const r = canvas.current?.getBoundingClientRect();
      return toDocument(
        cam.current,
        size.current,
        (clientX - (r?.left ?? 0)) * dpr(),
        (clientY - (r?.top ?? 0)) * dpr(),
      );
    },
    toView: (x, y) => toScreen(cam.current, size.current, x, y),
  };
}
