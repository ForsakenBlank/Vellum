import { useEffect } from "react";
import { create } from "zustand";

import type { MediaEvent } from "../bindings/MediaEvent";
import type { MediaStatus } from "../bindings/MediaStatus";
import type { Thumbnail } from "../bindings/Thumbnail";
import { inDesktopApp, ipc } from "../lib/ipc";

/** a thumbnail once it exists, or where it is on the way there */
export type ThumbnailState = Thumbnail | "pending" | "failed";

export function thumbnailKey(path: string, size: number): string {
  return `${String(size)}|${path}`;
}

interface MediaStore {
  /** null until the desktop app answers */
  status: MediaStatus | null;
  thumbnails: Record<string, ThumbnailState>;
  setStatus: (status: MediaStatus) => void;
  /** asks rust once per path and size; the answer comes back now or as an event */
  requestThumbnail: (path: string, size: number) => void;
  apply: (event: MediaEvent) => void;
}

/** the thumbnail table after a media event. pure, so it is tested without react. */
export function applyMediaEvent(
  thumbnails: Record<string, ThumbnailState>,
  event: MediaEvent,
): Record<string, ThumbnailState> {
  switch (event.type) {
    case "thumbnailReady":
      return { ...thumbnails, [thumbnailKey(event.path, event.size)]: event.thumbnail };
    case "failed": {
      if (event.what !== "thumbnail") return thumbnails;
      // the failure does not say which size, so every size still waiting gives up
      const next = { ...thumbnails };
      for (const [key, state] of Object.entries(next)) {
        if (state === "pending" && key.endsWith(`|${event.path}`)) next[key] = "failed";
      }
      return next;
    }
    default:
      return thumbnails;
  }
}

export const useMedia = create<MediaStore>()((set, get) => ({
  status: null,
  thumbnails: {},
  setStatus: (status) => {
    set({ status });
  },
  requestThumbnail: (path, size) => {
    const key = thumbnailKey(path, size);
    if (!inDesktopApp || key in get().thumbnails) return;
    set((s) => ({ thumbnails: { ...s.thumbnails, [key]: "pending" } }));
    ipc
      .mediaThumbnail(path, size)
      .then((thumbnail) => {
        // null means a job started; its event may even have arrived already
        if (thumbnail) set((s) => ({ thumbnails: { ...s.thumbnails, [key]: thumbnail } }));
      })
      .catch(() => {
        set((s) => ({ thumbnails: { ...s.thumbnails, [key]: "failed" } }));
      });
  },
  apply: (event) => {
    if (event.type === "statusChanged") {
      set({ status: event.status });
      return;
    }
    set((s) => ({ thumbnails: applyMediaEvent(s.thumbnails, event) }));
  },
}));

/** the thumbnail for `path`, asking for it the first time it is needed */
export function useThumbnail(path: string | null, size: number): ThumbnailState | undefined {
  const state = useMedia((s) => (path ? s.thumbnails[thumbnailKey(path, size)] : undefined));
  useEffect(() => {
    if (path) useMedia.getState().requestThumbnail(path, size);
  }, [path, size]);
  return state;
}
