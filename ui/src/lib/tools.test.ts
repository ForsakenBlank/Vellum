import { describe, expect, it } from "vitest";

import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { MediaInfo } from "../bindings/MediaInfo";
import {
  canRemux,
  clampTrim,
  commonTargets,
  deleteFrames,
  duplicateFrames,
  formatTime,
  goalBytes,
  latestPerFile,
  moveFrame,
  optionsFor,
  originalFrames,
  savings,
  setDelays,
  sizeLabel,
  snapToFrame,
  toolDefaults,
  withDefaults,
} from "./tools";

describe("options", () => {
  it("fills in what a session left out, keeping nulls it chose", () => {
    const options = optionsFor("video_to_gif", { anim: { fps: 12, width: null } });
    expect(options.anim.fps).toBe(12);
    expect(options.anim.width).toBeNull();
    expect(options.anim.colors).toBe(256);
    expect(options.fitToSize).toBe(false);
  });

  it("replaces arrays rather than merging them", () => {
    expect(withDefaults({ sizes: [1, 2, 3] }, { sizes: [9] })).toEqual({ sizes: [9] });
    expect(withDefaults({ a: 1 }, undefined)).toEqual({ a: 1 });
  });

  it("come from rust", () => {
    expect(toolDefaults.export_to_size.strategy).toBe("bestQuality");
    expect(toolDefaults.convert.icoSizes).toContain(256);
  });
});

describe("sizes", () => {
  it("read like people write them", () => {
    expect(sizeLabel({ amount: 10, unit: "MB" })).toBe("10 MB");
    expect(sizeLabel({ amount: 9.499999, unit: "MiB" })).toBe("9.5 MiB");
  });

  it("aim under the limit by the margin, as rust does", () => {
    expect(goalBytes({ amount: 10, unit: "MB" }, 3)).toBe(9_700_000);
    expect(goalBytes({ amount: 1, unit: "MiB" }, 10)).toBe(943_718);
    expect(goalBytes({ amount: 10, unit: "MB" }, 400)).toBe(5_000_000);
  });

  it("say how much smaller", () => {
    expect(savings(1000, 380)).toBe("62% smaller");
    expect(savings(1000, 1040)).toBe("4% bigger");
    expect(savings(0, 10)).toBeNull();
  });
});

describe("trim", () => {
  it("shows times with hundredths", () => {
    expect(formatTime(4.2)).toBe("0:04.20");
    expect(formatTime(62.5)).toBe("1:02.50");
    expect(formatTime(3723)).toBe("1:02:03.00");
  });

  it("lands on frames and keeps a minimum length", () => {
    expect(snapToFrame(1.01, 30)).toBeCloseTo(1.0);
    expect(snapToFrame(1.02, 30)).toBeCloseTo(31 / 30);
    expect(snapToFrame(-1, null)).toBe(0);
    expect(clampTrim(5, 4, 10)).toEqual({ start: 5, end: 5.1 });
    expect(clampTrim(-2, 20, 10)).toEqual({ start: 0, end: 10 });
  });
});

describe("frame editor", () => {
  const frames = originalFrames([100, 100, 200]);

  it("deletes, duplicates, moves and retimes", () => {
    expect(deleteFrames(frames, new Set([1])).map((f) => f.source)).toEqual([0, 2]);
    expect(duplicateFrames(frames, new Set([0])).map((f) => f.source)).toEqual([0, 0, 1, 2]);
    expect(moveFrame(frames, 2, 0).map((f) => f.source)).toEqual([2, 0, 1]);
    expect(moveFrame(frames, 0, 9).map((f) => f.source)).toEqual([1, 2, 0]);
    // delays snap to the hundredths gifs store
    expect(setDelays(frames, new Set([0, 2]), 333).map((f) => f.delayMs)).toEqual([330, 100, 330]);
    expect(setDelays(frames, new Set([1]), 1).map((f) => f.delayMs)[1]).toBe(10);
  });
});

describe("converter", () => {
  it("offers what every file can become", () => {
    expect(commonTargets(["video"])).toContain("gif");
    expect(commonTargets(["video", "audio"])).toEqual(["mp3", "m4a", "ogg", "wav", "flac"]);
    expect(commonTargets([])).toEqual([]);
  });

  const video = {
    video: { codec: "h264" },
    audio: [{ codec: "aac" }],
  } as unknown as MediaInfo;

  it("knows when streams can be copied", () => {
    const options = toolDefaults.convert;
    expect(canRemux(video, "mkv", options)).toBe(true);
    expect(canRemux(video, "webm", options)).toBe(false);
    expect(canRemux(video, "m4a", options)).toBe(true);
    expect(canRemux(video, "mp3", options)).toBe(false);
    expect(canRemux(video, "mkv", { ...options, rotate: 90 })).toBe(false);
    expect(canRemux(video, "png", options)).toBe(false);
  });
});

describe("jobs per file", () => {
  it("keeps the newest run of each file", () => {
    const job = (id: number, subject: string) => ({ id, subject }) as unknown as JobSnapshot;
    const latest = latestPerFile([job(9, "/a.mp4"), job(8, "/b.mp4"), job(3, "/a.mp4")]);
    expect(latest.get("/a.mp4")?.id).toBe(9);
    expect(latest.size).toBe(2);
  });
});
