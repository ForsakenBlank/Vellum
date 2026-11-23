import { create } from "zustand";

import type { Camera, Size } from "../../components/viewport/tool";

/** where each document's view looks, kept apart so only the navigator redraws */
export const useViewCamera = create<{
  views: Record<string, { camera: Camera; size: Size }>;
  set: (doc: string, camera: Camera, size: Size) => void;
}>()((set) => ({
  views: {},
  set: (doc, camera, size) => {
    set((s) => ({ views: { ...s.views, [doc]: { camera, size } } }));
  },
}));
