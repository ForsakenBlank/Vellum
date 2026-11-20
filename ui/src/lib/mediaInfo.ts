import type { MediaInfo } from "../bindings/MediaInfo";
import type { MetadataTag } from "../bindings/MetadataTag";
import type { VideoStream } from "../bindings/VideoStream";
import type { WaveformOverview } from "../bindings/WaveformOverview";
import { formatBytes, formatDuration } from "./format";

export interface InfoSection {
  title: string;
  rows: [label: string, value: string][];
}

const kindLabels: Record<MediaInfo["kind"], string> = {
  image: "Image",
  animation: "Animation",
  video: "Video",
  audio: "Audio",
  vector: "Vector drawing",
  unknown: "Unknown",
};

const hdrLabels = {
  hdr10: "HDR10",
  hdr10Plus: "HDR10+",
  hlg: "HLG",
  dolbyVision: "Dolby Vision",
  pq: "HDR (PQ)",
} as const;

/** short clips get a tenth of a second, longer ones minutes and seconds */
export function formatLength(seconds: number): string {
  return seconds < 60 ? `${seconds.toFixed(1)} s` : formatDuration(seconds);
}

/** 8000000 becomes "8 Mb/s". bit rates are in bits, so lowercase b. */
export function formatBitRate(bitsPerSecond: number): string {
  if (bitsPerSecond >= 1_000_000) {
    const mb = bitsPerSecond / 1_000_000;
    return `${mb >= 10 ? Math.round(mb) : mb.toFixed(1)} Mb/s`;
  }
  return `${Math.max(1, Math.round(bitsPerSecond / 1000))} kb/s`;
}

/** 29.97002997 becomes "29.97", 30 stays "30" */
export function formatFrameRate(fps: number): string {
  return Number.isInteger(fps) ? String(fps) : fps.toFixed(3).replace(/0+$/, "");
}

function channelsLabel(channels: number, layout: string | null): string {
  if (layout === "mono" || channels === 1) return "Mono";
  if (layout === "stereo" || channels === 2) return "Stereo";
  return layout ? `${String(channels)} (${layout})` : String(channels);
}

function videoRows(video: VideoStream): [string, string][] {
  const rows: [string, string][] = [["Codec", video.codecName || video.codec]];
  if (video.profile) rows.push(["Profile", video.profile]);
  if (video.frameRate !== null) {
    const fps = `${formatFrameRate(video.frameRate)} fps`;
    rows.push(["Frame rate", video.variableFrameRate ? `${fps}, variable` : fps]);
  } else if (video.variableFrameRate) {
    rows.push(["Frame rate", "Variable"]);
  }
  if (video.frameCount !== null) rows.push(["Frames", video.frameCount.toLocaleString("en")]);
  if (video.bitRate !== null) rows.push(["Bit rate", formatBitRate(video.bitRate)]);
  if (video.bitDepth !== null) rows.push(["Bit depth", `${String(video.bitDepth)} bit`]);
  if (video.pixelFormat) rows.push(["Pixel format", video.pixelFormat]);
  const colour = [video.color.primaries, video.color.transfer, video.color.space]
    .filter((part): part is string => part !== null)
    .join(", ");
  if (colour) rows.push(["Colour", colour]);
  if (video.color.range) {
    rows.push(["Range", video.color.range === "pc" ? "Full" : "Limited"]);
  }
  if (video.hdr) {
    const extra = [
      video.hdr.masteringDisplay,
      video.hdr.maxCll !== null ? `MaxCLL ${String(video.hdr.maxCll)} nits` : null,
    ].filter(Boolean);
    const label = hdrLabels[video.hdr.format];
    rows.push(["HDR", extra.length > 0 ? `${label} (${extra.join(", ")})` : label]);
  }
  if (video.rotation !== 0) rows.push(["Rotation", `${String(video.rotation)}° clockwise`]);
  return rows;
}

/** the info panel's sections, most useful first. metadata tags are separate. */
export function infoSections(info: MediaInfo): InfoSection[] {
  const general: [string, string][] = [
    ["Type", kindLabels[info.kind]],
    ["Format", info.format],
    ["Size", formatBytes(info.sizeBytes)],
  ];
  if (info.width !== null && info.height !== null) {
    general.push(["Dimensions", `${String(info.width)} × ${String(info.height)}`]);
  }
  if (info.durationSecs !== null) general.push(["Length", formatLength(info.durationSecs)]);
  if (info.bitRate !== null && info.kind !== "image") {
    general.push(["Bit rate", formatBitRate(info.bitRate)]);
  }
  if (info.image?.color) general.push(["Colour", info.image.color]);
  if (info.image?.frames != null) general.push(["Frames", String(info.image.frames)]);
  if (info.chapters > 0) general.push(["Chapters", String(info.chapters)]);

  const sections: InfoSection[] = [{ title: "General", rows: general }];
  if (info.video) sections.push({ title: "Video", rows: videoRows(info.video) });
  info.audio.forEach((audio, index) => {
    const rows: [string, string][] = [
      ["Codec", audio.codecName || audio.codec],
      ["Channels", channelsLabel(audio.channels, audio.channelLayout)],
    ];
    if (audio.sampleRate !== null) {
      rows.push(["Sample rate", `${(audio.sampleRate / 1000).toString()} kHz`]);
    }
    if (audio.bitRate !== null) rows.push(["Bit rate", formatBitRate(audio.bitRate)]);
    if (audio.language) rows.push(["Language", audio.language]);
    if (audio.title) rows.push(["Title", audio.title]);
    const title = info.audio.length > 1 ? `Audio ${String(index + 1)}` : "Audio";
    sections.push({ title, rows });
  });
  if (info.subtitles.length > 0) {
    sections.push({
      title: "Subtitles",
      rows: info.subtitles.map((sub, index) => [
        `Track ${String(index + 1)}`,
        [sub.language, sub.title, sub.codec].filter(Boolean).join(", "),
      ]),
    });
  }
  return sections;
}

/** tags grouped by where they came from, in the order they first appear */
export function tagGroups(tags: MetadataTag[]): [group: string, tags: MetadataTag[]][] {
  const groups = new Map<string, MetadataTag[]>();
  for (const tag of tags) {
    const list = groups.get(tag.group);
    if (list) list.push(tag);
    else groups.set(tag.group, [tag]);
  }
  return [...groups.entries()];
}

/** plain text for "Copy info", easy to paste into a bug report or a chat */
export function mediaInfoText(info: MediaInfo): string {
  const lines = [info.name, info.path, ""];
  for (const section of infoSections(info)) {
    lines.push(section.title);
    for (const [label, value] of section.rows) lines.push(`  ${label}: ${value}`);
    lines.push("");
  }
  for (const [group, tags] of tagGroups(info.tags)) {
    lines.push(`Metadata (${group})`);
    for (const tag of tags) lines.push(`  ${tag.key}: ${tag.value}`);
    lines.push("");
  }
  return lines.join("\n").trimEnd() + "\n";
}

/** image formats vellum can strip without re-encoding */
const STRIPPABLE_IMAGES = new Set(["JPEG", "PNG", "WebP"]);

/** why metadata cannot be stripped from this file, or null if it can */
export function stripBlocker(info: MediaInfo): string | null {
  if (info.kind === "video" || info.kind === "audio") return null;
  if (info.kind === "image" && STRIPPABLE_IMAGES.has(info.format)) return null;
  return `Stripping metadata works on JPEG, PNG, WebP, video and audio. Convert this ${info.format} file first.`;
}

/** one filled shape: the top edge left to right through the highest peaks, back along the lowest */
export function wavePath(view: WaveformOverview): string {
  const channels = Math.max(1, view.channels);
  const top: string[] = [];
  const bottom: string[] = [];
  for (let column = 0; column < view.columns; column++) {
    let low = 0;
    let high = 0;
    for (let channel = 0; channel < channels; channel++) {
      const at = (column * channels + channel) * 2;
      low = Math.min(low, view.peaks[at] ?? 0);
      high = Math.max(high, view.peaks[at + 1] ?? 0);
    }
    // keep silence visible as a hairline
    const y1 = 20 - Math.max(high * 20, 0.4);
    const y2 = 20 - Math.min(low * 20, -0.4);
    top.push(`${String(column)} ${y1.toFixed(2)}`);
    bottom.push(`${String(column)} ${y2.toFixed(2)}`);
  }
  return `M${top.join(" L")} L${bottom.reverse().join(" L")} Z`;
}
