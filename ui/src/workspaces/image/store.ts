import { create } from "zustand";

import type { BrushSettings } from "../../bindings/BrushSettings";
import type { Combine } from "../../bindings/Combine";
import type { FillSettings } from "../../bindings/FillSettings";
import type { GradientKind } from "../../bindings/GradientKind";

// the image editor's tools and their options. this is per window and only a convenience,
// like the palette's recent commands, so it is remembered in local storage rather than
// in rust: nothing here is part of a document.

export type ToolId =
  | "move"
  | "marquee"
  | "lasso"
  | "wand"
  | "eyedropper"
  | "brush"
  | "eraser"
  | "gradient"
  | "hand"
  | "zoom";

export type MarqueeShape = "rect" | "ellipse";
export type LassoKind = "free" | "polygon" | "magnetic";
export type WandKind = "wand" | "quick";
export type BrushKind = "brush" | "pencil";
export type FillKind = "gradient" | "bucket";

export type Rgb = [number, number, number];

export interface ToolState {
  tool: ToolId;
  marquee: MarqueeShape;
  lasso: LassoKind;
  wand: WandKind;
  brushKind: BrushKind;
  fill: FillKind;
  brush: BrushSettings;
  eraser: BrushSettings;
  selectMode: Combine;
  feather: number;
  antiAlias: boolean;
  tolerance: number;
  contiguous: boolean;
  sampleAll: boolean;
  quickSize: number;
  magneticWidth: number;
  magneticContrast: number;
  gradientKind: GradientKind;
  gradientReverse: boolean;
  gradientDither: boolean;
  gradientOpacity: number;
  /** the gradient fades the foreground out to nothing instead of to the background */
  gradientToClear: boolean;
  bucket: Omit<FillSettings, "color">;
  sampleSize: number;
  sampleLayers: "current" | "all";
  foreground: Rgb;
  background: Rgb;
}

export const defaultBrush: BrushSettings = {
  tool: "brush",
  size: 30,
  hardness: 0.8,
  opacity: 1,
  flow: 1,
  spacing: 0.1,
  smoothing: 0.1,
  scatter: 0,
  count: 1,
  angle: 0,
  roundness: 1,
  textureDepth: 0,
  textureScale: 100,
  pressureSize: true,
  pressureOpacity: false,
  pressureFlow: false,
  minSize: 0,
  tilt: false,
  mode: "normal",
  color: [0, 0, 0],
  background: [1, 1, 1],
  seed: 1,
};

export const defaultTools: ToolState = {
  tool: "brush",
  marquee: "rect",
  lasso: "free",
  wand: "wand",
  brushKind: "brush",
  fill: "gradient",
  brush: defaultBrush,
  eraser: { ...defaultBrush, tool: "eraser", hardness: 0.5, size: 40 },
  selectMode: "replace",
  feather: 0,
  antiAlias: true,
  tolerance: 32,
  contiguous: true,
  sampleAll: false,
  quickSize: 20,
  magneticWidth: 10,
  magneticContrast: 0.3,
  gradientKind: "linear",
  gradientReverse: false,
  gradientDither: true,
  gradientOpacity: 1,
  gradientToClear: false,
  bucket: {
    tolerance: 32,
    contiguous: true,
    antiAlias: true,
    sampleAll: false,
    opacity: 1,
    mode: "normal",
  },
  sampleSize: 1,
  sampleLayers: "all",
  foreground: [0, 0, 0],
  background: [1, 1, 1],
};

const KEY = "vellum.imageTools";

function load(): ToolState {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (raw && typeof raw === "object") {
      const saved = raw as Partial<ToolState>;
      return {
        ...defaultTools,
        ...saved,
        brush: { ...defaultTools.brush, ...saved.brush },
        eraser: { ...defaultTools.eraser, ...saved.eraser },
        bucket: { ...defaultTools.bucket, ...saved.bucket },
      };
    }
  } catch {
    // storage can be blocked or hold something old; the defaults are fine
  }
  return defaultTools;
}

interface Store extends ToolState {
  set: (patch: Partial<ToolState>) => void;
  setBrush: (patch: Partial<BrushSettings>) => void;
}

export const useImageTools = create<Store>()((set) => ({
  ...load(),
  set: (patch) => {
    set(patch);
  },
  setBrush: (patch) => {
    set((s) =>
      s.tool === "eraser"
        ? { eraser: { ...s.eraser, ...patch } }
        : { brush: { ...s.brush, ...patch } },
    );
  },
}));

useImageTools.subscribe((state) => {
  try {
    // json leaves the store's functions out by itself
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // the options are only a convenience
  }
});
