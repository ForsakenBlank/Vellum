import type { HdrInfo } from "../bindings/HdrInfo";
import type { MediaInfo } from "../bindings/MediaInfo";
import type { VideoInfo } from "../bindings/VideoInfo";
import { formatBytes } from "./format";

export interface InfoRow {
  label: string;
  value: string;
}

export interface InfoSection {
  title: string;
  rows: InfoRow[];
}

/** "1:05", "1:02:03", "0:04.5" for short clips */
export function formatClock(totalSeconds: number): string {
  const seconds = Math.max(0, totalSeconds);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  const secondsText =
    seconds < 10 && rest % 1 !== 0
      ? rest.toFixed(1).padStart(4, "0")
      : String(Math.floor(rest)).padStart(2, "0");
  return hours > 0
    ? `${String(hours)}:${String(minutes).padStart(2, "0")}:${secondsText}`
    : `${String(minutes)}:${secondsText}`;
}

/** 5_200_000 becomes "5.2 Mb/s" */
export function formatBitRate(bitsPerSecond: number): string {
  if (bitsPerSecond >= 1_000_000) return `${(bitsPerSecond / 1_000_000).toFixed(1)} Mb/s`;
  return `${String(Math.round(bitsPerSecond / 1000))} kb/s`;
}

/** 29.97002997 becomes "29.97", 25 stays "25" */
export function formatFps(fps: number): string {
  return String(Math.round(fps * 100) / 100);
}

function size(width: number, height: number): string {
  return `${String(width)} × ${String(height)}`;
}

function channelsText(channels: number, layout: string | null): string {
  if (layout === "mono" || channels === 1) return "Mono";
  if (layout === "stereo" || channels === 2) return "Stereo";
  return layout ? `${layout} (${String(channels)} channels)` : `${String(channels)} channels`;
}

function hdrText(hdr: HdrInfo): string {
  const parts: string[] = [hdrLabel(hdr.format)];
  if (hdr.masteringMaxNits)
    parts.push(`mastered at ${String(Math.round(hdr.masteringMaxNits))} nits`);
  if (hdr.maxContentNits) parts.push(`peak ${String(hdr.maxContentNits)} nits`);
  return parts.join(", ");
}

function hdrLabel(format: HdrInfo["format"]): string {
  switch (format) {
    case "hdr10":
      return "HDR10";
    case "hdr10Plus":
      return "HDR10+";
    case "hlg":
      return "HLG";
    case "dolbyVision":
      return "Dolby Vision";
  }
}

function videoColour(video: VideoInfo): string | null {
  const parts = [
    video.bitDepth ? `${String(video.bitDepth)}-bit` : null,
    video.color.primaries ?? video.color.space,
    video.color.range === "pc" ? "full range" : video.color.range === "tv" ? "limited range" : null,
  ].filter((p): p is string => Boolean(p));
  return parts.length > 0 ? parts.join(", ") : null;
}

/** the info panel's sections, in reading order */
export function describeMedia(info: MediaInfo): InfoSection[] {
  const sections: InfoSection[] = [];
  const general: InfoRow[] = [
    { label: "Format", value: info.formatLabel },
    { label: "Size", value: formatBytes(info.sizeBytes) },
  ];
  if (info.durationSecs !== null) {
    general.push({ label: "Length", value: formatClock(info.durationSecs) });
  }
  if (info.bitRate !== null && info.kind !== "image") {
    general.push({ label: "Bit rate", value: formatBitRate(info.bitRate) });
  }
  sections.push({ title: "File", rows: general });

  const image = info.image;
  if (image) {
    const rows: InfoRow[] = [
      { label: "Dimensions", value: size(image.displayWidth, image.displayHeight) },
    ];
    if (image.displayWidth !== image.width) {
      rows.push({
        label: "Stored as",
        value: `${size(image.width, image.height)}, turned upright`,
      });
    }
    if (image.frames > 1) {
      rows.push({ label: "Frames", value: String(image.frames) });
      if (image.loopCount !== null) {
        rows.push({
          label: "Plays",
          value:
            image.loopCount === 0
              ? "Forever"
              : image.loopCount === 1
                ? "Once"
                : `${String(image.loopCount)} times`,
        });
      }
    }
    const colour = [image.colorModel, image.bitDepth ? `${String(image.bitDepth)}-bit` : null]
      .filter((p): p is string => Boolean(p))
      .join(", ");
    if (colour) rows.push({ label: "Colour", value: colour });
    rows.push({ label: "Transparency", value: image.hasAlpha ? "Yes" : "No" });
    rows.push({ label: "Colour profile", value: image.hasIccProfile ? "Embedded" : "None" });
    if (image.dpi)
      rows.push({ label: "Resolution", value: `${String(Math.round(image.dpi))} dpi` });
    sections.push({ title: info.kind === "animation" ? "Animation" : "Image", rows });
  }

  const video = info.video;
  if (video && !image) {
    const rows: InfoRow[] = [
      {
        label: "Codec",
        value: video.profile ? `${video.codecLabel} (${video.profile})` : video.codecLabel,
      },
      { label: "Resolution", value: size(video.displayWidth, video.displayHeight) },
    ];
    if (video.rotation !== 0) {
      rows.push({ label: "Rotation", value: `${String(video.rotation)}° (filmed sideways)` });
    }
    if (video.fps !== null) {
      const variable = video.frameRateMode === "variable";
      rows.push({
        label: "Frame rate",
        value: `${formatFps(video.fps)} fps${variable ? ", variable" : ""}`,
      });
    }
    if (video.frameCount !== null) rows.push({ label: "Frames", value: String(video.frameCount) });
    if (video.bitRate !== null)
      rows.push({ label: "Bit rate", value: formatBitRate(video.bitRate) });
    const colour = videoColour(video);
    if (colour) rows.push({ label: "Colour", value: colour });
    if (video.hdr) rows.push({ label: "HDR", value: hdrText(video.hdr) });
    if (video.interlaced) rows.push({ label: "Scan", value: "Interlaced" });
    sections.push({ title: "Video", rows });
  }

  const audio = info.audio;
  if (audio) {
    const rows: InfoRow[] = [
      { label: "Codec", value: audio.codecLabel },
      { label: "Channels", value: channelsText(audio.channels, audio.channelLayout) },
      { label: "Sample rate", value: `${String(audio.sampleRate / 1000)} kHz` },
    ];
    if (audio.bitDepth) rows.push({ label: "Bit depth", value: `${String(audio.bitDepth)}-bit` });
    if (audio.bitRate !== null)
      rows.push({ label: "Bit rate", value: formatBitRate(audio.bitRate) });
    sections.push({ title: "Audio", rows });
  }

  const others = info.streams.filter((s) => s.kind === "subtitle").length;
  if (others > 0) {
    sections.push({
      title: "Other",
      rows: [{ label: "Subtitles", value: `${String(others)} track${others === 1 ? "" : "s"}` }],
    });
  }
  return sections;
}

/** everything in the panel as plain text, for "Copy info" */
export function mediaInfoText(info: MediaInfo): string {
  const lines = [info.path];
  for (const section of describeMedia(info)) {
    lines.push("", section.title);
    for (const row of section.rows) lines.push(`  ${row.label}: ${row.value}`);
  }
  if (info.tags.length > 0) {
    lines.push("", "Metadata");
    for (const tag of info.tags) lines.push(`  ${tag.group} / ${tag.key}: ${tag.value}`);
  }
  return lines.join("\n");
}
