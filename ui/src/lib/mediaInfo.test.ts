import { describe, expect, it } from "vitest";

import type { MediaInfo } from "../bindings/MediaInfo";
import { describeMedia, formatBitRate, formatClock, formatFps, mediaInfoText } from "./mediaInfo";

function phoneVideo(): MediaInfo {
  return {
    path: "/clips/beach.mov",
    kind: "video",
    format: "mov",
    formatLabel: "QuickTime / MPEG-4",
    sizeBytes: 48_000_000,
    durationSecs: 65.2,
    bitRate: 5_900_000,
    video: {
      streamIndex: 0,
      codec: "hevc",
      codecLabel: "H.265 / HEVC",
      profile: "Main 10",
      width: 1920,
      height: 1080,
      displayWidth: 1080,
      displayHeight: 1920,
      rotation: 90,
      frameRate: { num: 30000, den: 1001 },
      fps: 29.97002997,
      frameRateMode: "variable",
      frameCount: 1954,
      bitRate: 5_700_000,
      pixelFormat: "yuv420p10le",
      bitDepth: 10,
      color: { space: "bt2020nc", primaries: "bt2020", transfer: "arib-std-b67", range: "tv" },
      hdr: {
        format: "hlg",
        masteringMaxNits: null,
        masteringMinNits: null,
        maxContentNits: null,
        maxFrameAverageNits: null,
      },
      interlaced: false,
    },
    audio: {
      streamIndex: 1,
      codec: "aac",
      codecLabel: "AAC",
      profile: "LC",
      channels: 2,
      channelLayout: "stereo",
      sampleRate: 48000,
      sampleFormat: "fltp",
      bitDepth: null,
      bitRate: 190_000,
      durationSecs: 65.2,
    },
    image: null,
    streams: [],
    tags: [
      { group: "File", key: "location", value: "+51.5-000.1/", sensitive: true },
      { group: "File", key: "com.apple.quicktime.model", value: "iPhone", sensitive: true },
    ],
    warnings: [],
  };
}

describe("formatters", () => {
  it("reads like a media player", () => {
    expect(formatClock(65.2)).toBe("1:05");
    expect(formatClock(3723)).toBe("1:02:03");
    expect(formatClock(4.5)).toBe("0:04.5");
    expect(formatBitRate(5_900_000)).toBe("5.9 Mb/s");
    expect(formatBitRate(190_000)).toBe("190 kb/s");
    expect(formatFps(29.97002997)).toBe("29.97");
    expect(formatFps(25)).toBe("25");
  });
});

describe("describeMedia", () => {
  it("shows a phone video the way it plays, not the way it is stored", () => {
    const sections = describeMedia(phoneVideo());
    expect(sections.map((s) => s.title)).toEqual(["File", "Video", "Audio"]);
    const video = Object.fromEntries(
      sections[1]?.rows.map((r) => [r.label, r.value]) ?? [],
    ) as Record<string, string>;
    expect(video.Resolution).toBe("1080 × 1920");
    expect(video.Rotation).toBe("90° (filmed sideways)");
    expect(video["Frame rate"]).toBe("29.97 fps, variable");
    expect(video.Colour).toBe("10-bit, bt2020, limited range");
    expect(video.HDR).toBe("HLG");
  });

  it("describes animations with their frames and loops", () => {
    const gif: MediaInfo = {
      ...phoneVideo(),
      kind: "animation",
      formatLabel: "GIF",
      video: null,
      audio: null,
      durationSecs: 2,
      tags: [],
      image: {
        width: 320,
        height: 240,
        displayWidth: 320,
        displayHeight: 240,
        frames: 20,
        loopCount: 0,
        hasAlpha: true,
        bitDepth: 8,
        colorModel: "RGB",
        hasIccProfile: false,
        orientation: 1,
        dpi: null,
      },
    };
    const sections = describeMedia(gif);
    expect(sections[1]?.title).toBe("Animation");
    const rows = Object.fromEntries(sections[1]?.rows.map((r) => [r.label, r.value]) ?? []);
    expect(rows).toMatchObject({ Frames: "20", Plays: "Forever", Colour: "RGB, 8-bit" });
  });

  it("copies everything including metadata as text", () => {
    const text = mediaInfoText(phoneVideo());
    expect(text.split("\n")[0]).toBe("/clips/beach.mov");
    expect(text).toContain("  Codec: H.265 / HEVC (Main 10)");
    expect(text).toContain("  File / location: +51.5-000.1/");
  });
});
