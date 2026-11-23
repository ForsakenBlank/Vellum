import { create } from "zustand";

import type { Interpolation } from "../../bindings/Interpolation";
import type { TransformTarget } from "../../bindings/TransformTarget";
import type { Box, CropBox, HandleMode, Pt, Quad } from "./transformMath";

// what the window keeps while a free transform or a crop is being set up. none of it is
// in the document until it is committed, which is one command, so cancelling leaves no
// trace. the view aids (rulers, grid, guides, snapping) are per window and remembered
// like the tool options.

export interface TransformSession {
  doc: string;
  target: TransformTarget;
  source: Box;
  corners: Quad;
  /** sixteen control points once the warp is on */
  warp: Pt[] | null;
  /** what corner and edge drags do */
  mode: HandleMode;
  interpolation: Interpolation;
  /** where rotation turns about */
  pivot: Pt;
}

export interface CropSession {
  doc: string;
  box: CropBox;
  /** four free corners for a perspective crop */
  quad: Quad | null;
  /** width over height, 0 when free */
  aspect: number;
  deleteOutside: boolean;
  /** a line being drawn to level the picture */
  straighten: boolean;
}

interface Sessions {
  transform: TransformSession | null;
  crop: CropSession | null;
  setTransform: (t: TransformSession | null) => void;
  patchTransform: (patch: Partial<TransformSession>) => void;
  setCrop: (c: CropSession | null) => void;
  patchCrop: (patch: Partial<CropSession>) => void;
}

export const useSessions = create<Sessions>()((set) => ({
  transform: null,
  crop: null,
  setTransform: (transform) => {
    set({ transform });
  },
  patchTransform: (patch) => {
    set((s) => (s.transform ? { transform: { ...s.transform, ...patch } } : {}));
  },
  setCrop: (crop) => {
    set({ crop });
  },
  patchCrop: (patch) => {
    set((s) => (s.crop ? { crop: { ...s.crop, ...patch } } : {}));
  },
}));

export interface ViewAids {
  rulers: boolean;
  grid: boolean;
  guides: boolean;
  lockGuides: boolean;
  /** snapping on at all */
  snap: boolean;
  snapGuides: boolean;
  snapGrid: boolean;
  snapCanvas: boolean;
  /** smart guides: other layers' edges and middles */
  snapLayers: boolean;
  /** grid line spacing in document pixels */
  gridSize: number;
  gridDivisions: number;
}

export const defaultAids: ViewAids = {
  rulers: false,
  grid: false,
  guides: true,
  lockGuides: false,
  snap: true,
  snapGuides: true,
  snapGrid: true,
  snapCanvas: true,
  snapLayers: true,
  gridSize: 100,
  gridDivisions: 4,
};

const KEY = "vellum.imageView";

function load(): ViewAids {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (raw && typeof raw === "object") return { ...defaultAids, ...(raw as Partial<ViewAids>) };
  } catch {
    // storage can be blocked or hold something old; the defaults are fine
  }
  return defaultAids;
}

interface AidsStore extends ViewAids {
  set: (patch: Partial<ViewAids>) => void;
  toggle: (key: "rulers" | "grid" | "guides" | "lockGuides" | "snap") => void;
}

export const useAids = create<AidsStore>()((set) => ({
  ...load(),
  set: (patch) => {
    set(patch);
  },
  toggle: (key) => {
    set((s) => ({ [key]: !s[key] }));
  },
}));

useAids.subscribe((state) => {
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // only a convenience
  }
});
