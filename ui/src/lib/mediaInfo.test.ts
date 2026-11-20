import { describe, expect, it } from "vitest";

import type { MediaInfo } from "../bindings/MediaInfo";
import {
  formatBitRate,
  formatFrameRate,
  infoSections,
  mediaInfoText,
  stripBlocker,
  tagGroups,
  wavePath,
} from "./mediaInfo";

function video(): MediaInfo {
  return {
    path: "/clips/beach.mp4",
    name: "beach.mp4",
    kind: "video",
    format: "MP4 / QuickTime",
    sizeBytes: 12_500_000,
    modifiedMs: null,
    width: 2160,
    height: 3840,
    durationSecs: 12.5,
    bitRate: 8_000_000,
    video: {
      codec: "hevc",
      codecName: "H.265 / HEVC",
      profile: "Main 10",
      pixelFormat: "yuv420p10le",
      bitDepth: 10,
      codedWidth: 3840,
      codedHeight: 2160,
      frameRate: 30000 / 1001,
      frameRateText: "30000/1001",
      variableFrameRate: true,
      frameCount: 375,
      bitRate: 7_800_000,
      rotation: 90,
      color: { space: "bt2020nc", primaries: "bt2020", transfer: "arib-std-b67", range: "tv" },
      hdr: { format: "hlg", masteringDisplay: null, maxCll: null, maxFall: null },
    },
    audio: [
      {
        codec: "aac",
        codecName: "AAC (Advanced Audio Coding)",
        channels: 2,
        channelLayout: "stereo",
        sampleRate: 48000,
        bitRate: 192_000,
        language: null,
        title: null,
      },
    ],
    subtitles: [],
    image: null,
    chapters: 0,
    tags: [
      { group: "File", key: "encoder", value: "Lavf60" },
      { group: "Video", key: "handler_name", value: "Core Media Video" },
      { group: "File", key: "location", value: "+51.5+000.1/" },
    ],
    hasLocation: true,
  };
}

describe("media info", () => {
  it("formats rates for people", () => {
    expect(formatBitRate(8_000_000)).toBe("8.0 Mb/s");
    expect(formatBitRate(25_000_000)).toBe("25 Mb/s");
    expect(formatBitRate(128_000)).toBe("128 kb/s");
    expect(formatFrameRate(30)).toBe("30");
    expect(formatFrameRate(30000 / 1001)).toBe("29.97");
    expect(formatFrameRate(23.976)).toBe("23.976");
  });

  it("builds sections with the details that matter", () => {
    const sections = infoSections(video());
    expect(sections.map((s) => s.title)).toEqual(["General", "Video", "Audio"]);
    const general = Object.fromEntries(sections[0]?.rows ?? []);
    expect(general.Dimensions).toBe("2160 × 3840");
    expect(general.Length).toBe("12.5 s");
    const videoRows = Object.fromEntries(sections[1]?.rows ?? []);
    expect(videoRows["Frame rate"]).toBe("29.97 fps, variable");
    expect(videoRows.HDR).toBe("HLG");
    expect(videoRows.Rotation).toBe("90° clockwise");
    expect(videoRows.Range).toBe("Limited");
    const audio = Object.fromEntries(sections[2]?.rows ?? []);
    expect(audio.Channels).toBe("Stereo");
    expect(audio["Sample rate"]).toBe("48 kHz");
  });

  it("groups tags in the order they appear", () => {
    const groups = tagGroups(video().tags);
    expect(groups.map(([g, tags]) => [g, tags.length])).toEqual([
      ["File", 2],
      ["Video", 1],
    ]);
  });

  it("copies as readable text", () => {
    const text = mediaInfoText(video());
    expect(text.startsWith("beach.mp4\n/clips/beach.mp4\n\nGeneral\n  Type: Video\n")).toBe(true);
    expect(text).toContain("Metadata (File)\n  encoder: Lavf60\n  location: +51.5+000.1/");
    expect(text.endsWith("\n")).toBe(true);
  });

  it("only offers to strip formats that can be stripped without re-encoding", () => {
    expect(stripBlocker(video())).toBeNull();
    expect(stripBlocker({ ...video(), kind: "image", format: "JPEG" })).toBeNull();
    expect(stripBlocker({ ...video(), kind: "image", format: "TIFF" })).toContain(
      "Convert this TIFF",
    );
    expect(stripBlocker({ ...video(), kind: "animation", format: "GIF" })).not.toBeNull();
  });

  it("draws a waveform as one closed shape, silence as a hairline", () => {
    const path = wavePath({ channels: 1, columns: 2, peaks: [-0.5, 0.5, 0, 0], durationSecs: 1 });
    expect(path).toBe("M0 10.00 L1 19.60 L1 20.40 L0 30.00 Z");
  });
});
