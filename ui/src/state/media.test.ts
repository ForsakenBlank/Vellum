import { describe, expect, it } from "vitest";

import { applyMediaEvent, thumbnailKey, type ThumbnailState } from "./media";

describe("applyMediaEvent", () => {
  const ready = { file: "/cache/blobs/thumb-1.jpg", width: 320, height: 240 };

  it("stores a finished thumbnail under its path and size", () => {
    const next = applyMediaEvent(
      { [thumbnailKey("/a.png", 320)]: "pending" },
      { type: "thumbnailReady", path: "/a.png", size: 320, thumbnail: ready },
    );
    expect(next[thumbnailKey("/a.png", 320)]).toEqual(ready);
  });

  it("marks every waiting size of a failed file, and nothing else", () => {
    const before: Record<string, ThumbnailState> = {
      [thumbnailKey("/a.png", 320)]: "pending",
      [thumbnailKey("/a.png", 64)]: "pending",
      [thumbnailKey("/b.png", 320)]: "pending",
      [thumbnailKey("/sub/a.png", 320)]: "pending",
      [thumbnailKey("/a.png", 128)]: ready,
    };
    const next = applyMediaEvent(before, {
      type: "failed",
      path: "/a.png",
      what: "thumbnail",
      error: "damaged",
    });
    expect(next[thumbnailKey("/a.png", 320)]).toBe("failed");
    expect(next[thumbnailKey("/a.png", 64)]).toBe("failed");
    expect(next[thumbnailKey("/b.png", 320)]).toBe("pending");
    expect(next[thumbnailKey("/sub/a.png", 320)]).toBe("pending");
    expect(next[thumbnailKey("/a.png", 128)]).toEqual(ready);
  });

  it("ignores failures of other kinds of work", () => {
    const before: Record<string, ThumbnailState> = { [thumbnailKey("/a.mp4", 320)]: "pending" };
    const next = applyMediaEvent(before, {
      type: "failed",
      path: "/a.mp4",
      what: "waveform",
      error: "no sound",
    });
    expect(next).toBe(before);
  });
});
