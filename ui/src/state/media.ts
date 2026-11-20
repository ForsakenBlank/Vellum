import { useEffect } from "react";
import { create } from "zustand";

import type { AssetKind } from "../bindings/AssetKind";
import type { AssetState } from "../bindings/AssetState";
import type { MediaAsset } from "../bindings/MediaAsset";
import type { MediaCapabilities } from "../bindings/MediaCapabilities";
import type { MediaInfo } from "../bindings/MediaInfo";
import { inDesktopApp, ipc } from "../lib/ipc";

/** the filmstrip every view asks for, so they all share one cached image */
export const FILMSTRIP_FRAMES = 8;
export const FILMSTRIP_HEIGHT = 72;

export type ProbeState =
  { state: "loading" } | { state: "ready"; info: MediaInfo } | { state: "failed"; error: string };

interface MediaStore {
  /** previews by `assetKey` */
  assets: Record<string, AssetState>;
  /** probe results by path */
  infos: Record<string, ProbeState>;
  capabilities: MediaCapabilities | null;
  /** goes up when the cache is cleaned, so views ask for their previews again */
  generation: number;
  apply: (asset: MediaAsset) => void;
  setInfo: (path: string, state: ProbeState) => void;
  setCapabilities: (capabilities: MediaCapabilities) => void;
  forget: () => void;
}

export function assetKey(kind: AssetKind, source: string): string {
  return `${kind}:${source}`;
}

/**
 * the ipc reply to a request and the event when the job is done can arrive in either
 * order, so a late "pending" reply must not hide a result that already came in
 */
export function mergeAsset(previous: AssetState | undefined, next: AssetState): AssetState {
  if (next.state === "pending" && previous && previous.state !== "pending") return previous;
  return next;
}

export const useMedia = create<MediaStore>()((set) => ({
  assets: {},
  infos: {},
  capabilities: null,
  generation: 0,
  apply: (asset) => {
    set((s) => {
      const key = assetKey(asset.kind, asset.source);
      return { assets: { ...s.assets, [key]: mergeAsset(s.assets[key], asset.state) } };
    });
  },
  setInfo: (path, state) => {
    set((s) => ({ infos: { ...s.infos, [path]: state } }));
  },
  setCapabilities: (capabilities) => {
    set({ capabilities });
  },
  forget: () => {
    asked.clear();
    set((s) => ({ assets: {}, infos: {}, generation: s.generation + 1 }));
  },
}));

/** previews this window already asked rust for, so a rerender never asks twice */
const asked = new Set<string>();

function ask(kind: AssetKind, source: string): Promise<MediaAsset> {
  switch (kind) {
    case "thumbnail":
      return ipc.mediaThumbnail(source);
    case "filmstrip":
      return ipc.mediaFilmstrip(source, FILMSTRIP_FRAMES, FILMSTRIP_HEIGHT);
    case "waveform":
      return ipc.mediaWaveform(source);
  }
}

export function requestAsset(kind: AssetKind, source: string) {
  const key = assetKey(kind, source);
  if (asked.has(key) || !inDesktopApp) return;
  asked.add(key);
  const { apply } = useMedia.getState();
  ask(kind, source)
    .then(apply)
    .catch((err: unknown) => {
      apply({ source, kind, state: { state: "failed", error: String(err) } });
    });
}

/** a preview of `source`, asked for when first shown. undefined until rust answers. */
export function useAsset(kind: AssetKind, source: string | null): AssetState | undefined {
  const state = useMedia((s) => (source ? s.assets[assetKey(kind, source)] : undefined));
  const generation = useMedia((s) => s.generation);
  useEffect(() => {
    if (source) requestAsset(kind, source);
  }, [kind, source, generation]);
  return state;
}

/** everything rust can tell about a file, probed once per window and kept */
export function useMediaInfo(path: string | null): ProbeState | undefined {
  const state = useMedia((s) => (path ? s.infos[path] : undefined));
  const generation = useMedia((s) => s.generation);
  useEffect(() => {
    if (!path || !inDesktopApp) return;
    const { infos, setInfo } = useMedia.getState();
    if (infos[path]) return;
    setInfo(path, { state: "loading" });
    ipc
      .probeMedia(path)
      .then((info) => {
        setInfo(path, { state: "ready", info });
      })
      .catch((err: unknown) => {
        setInfo(path, { state: "failed", error: String(err) });
      });
  }, [path, generation]);
  return state;
}

/** the file a ready asset points at, if any */
export function assetFile(state: AssetState | undefined): string | null {
  return state?.state === "ready" ? state.file : null;
}
