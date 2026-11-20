import { beforeEach, describe, expect, it } from "vitest";

import { assetKey, mergeAsset, useMedia } from "./media";

describe("media previews", () => {
  beforeEach(() => {
    useMedia.getState().forget();
  });

  it("never lets a late pending reply hide a finished preview", () => {
    const ready = { state: "ready", file: "/cache/1.jpg", meta: null } as const;
    expect(mergeAsset(ready, { state: "pending" })).toBe(ready);
    expect(mergeAsset(undefined, { state: "pending" })).toEqual({ state: "pending" });
    const failed = { state: "failed", error: "broken" } as const;
    expect(mergeAsset({ state: "pending" }, failed)).toBe(failed);
    expect(mergeAsset(failed, ready)).toBe(ready);
  });

  it("keys previews by kind and source", () => {
    const { apply } = useMedia.getState();
    apply({
      source: "/a.mp4",
      kind: "thumbnail",
      state: { state: "ready", file: "/c/1.jpg", meta: null },
    });
    apply({ source: "/a.mp4", kind: "thumbnail", state: { state: "pending" } });
    apply({ source: "/a.mp4", kind: "waveform", state: { state: "pending" } });
    const { assets } = useMedia.getState();
    expect(assets[assetKey("thumbnail", "/a.mp4")]).toEqual({
      state: "ready",
      file: "/c/1.jpg",
      meta: null,
    });
    expect(assets[assetKey("waveform", "/a.mp4")]).toEqual({ state: "pending" });
  });

  it("forgets everything after the cache is cleaned", () => {
    const before = useMedia.getState().generation;
    useMedia.getState().apply({ source: "/a.png", kind: "thumbnail", state: { state: "pending" } });
    useMedia.getState().setInfo("/a.png", { state: "loading" });
    useMedia.getState().forget();
    const after = useMedia.getState();
    expect(after.assets).toEqual({});
    expect(after.infos).toEqual({});
    expect(after.generation).toBe(before + 1);
  });
});
