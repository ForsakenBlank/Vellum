// pure helpers for the quick tool views: their options, sizes, times, the frame editor's
// list and the converter's targets. kept free of react so they are tested on their own.

import type { ConvertOptions } from "../bindings/ConvertOptions";
import type { ConvertTarget } from "../bindings/ConvertTarget";
import type { EditedFrame } from "../bindings/EditedFrame";
import type { GifMakerOptions } from "../bindings/GifMakerOptions";
import type { GifToolOptions } from "../bindings/GifToolOptions";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { MediaInfo } from "../bindings/MediaInfo";
import type { MediaKind } from "../bindings/MediaKind";
import type { RenamePlan } from "../bindings/RenamePlan";
import type { SizeOptions } from "../bindings/SizeOptions";
import type { SizePreset } from "../bindings/SizePreset";
import type { SizeUnit } from "../bindings/SizeUnit";
import type { TargetSize } from "../bindings/TargetSize";
import type { ToolResult } from "../bindings/ToolResult";
import defaults from "../bindings/toolDefaults.json";

/** the options each quick tool keeps in its document */
export interface ToolOptions {
  export_to_size: SizeOptions;
  video_to_gif: GifMakerOptions;
  gif_tools: GifToolOptions;
  convert: ConvertOptions;
}

export type RunnableTool = keyof ToolOptions;

const runnable: RunnableTool[] = ["export_to_size", "video_to_gif", "gif_tools", "convert"];

/** quick tools that work today, the rest arrive in later phases */
export function isRunnable(tool: string | null): tool is RunnableTool {
  return runnable.includes(tool as RunnableTool);
}

/** written by a rust test from the real defaults, see crates/vellum-export/src/service.rs */
export const toolDefaults = defaults as unknown as ToolOptions & {
  convertTargets: Record<string, ConvertTarget[]>;
  minSafeDelayMs: number;
};

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * fills in whatever `stored` leaves out from `base`, all the way down. stored values win,
 * including null, which means "use the setting" for several options.
 */
export function withDefaults<T>(base: T, stored: unknown): T {
  if (!isPlainObject(base) || !isPlainObject(stored)) {
    return stored === undefined ? base : (stored as T);
  }
  const out: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(stored)) {
    out[key] = key in base ? withDefaults(base[key], value) : value;
  }
  return out as T;
}

export function optionsFor<K extends RunnableTool>(tool: K, stored: unknown): ToolOptions[K] {
  return withDefaults(toolDefaults[tool], stored);
}

export const unitBytes: Record<SizeUnit, number> = { MB: 1_000_000, MiB: 1_048_576 };

/** "10 MB", "9.5 MiB" */
export function sizeLabel(target: TargetSize): string {
  const amount = Math.round(target.amount * 100) / 100;
  return `${String(amount)} ${target.unit}`;
}

/** the bytes to aim for, the limit less the safety margin, as rust works it out */
export function goalBytes(target: TargetSize, marginPercent: number): number {
  const margin = Math.min(50, Math.max(0, marginPercent));
  return Math.floor(target.amount * unitBytes[target.unit] * (1 - margin / 100));
}

export function samePreset(preset: SizePreset, target: TargetSize): boolean {
  return preset.unit === target.unit && Math.abs(preset.amount - target.amount) < 1e-9;
}

/** "0:04.20", "1:02:03.00": minutes and seconds with hundredths, for trim handles */
export function formatTime(seconds: number): string {
  const total = Math.max(0, seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total - hours * 3600 - minutes * 60;
  const secs = rest.toFixed(2).padStart(5, "0");
  return hours > 0
    ? `${String(hours)}:${String(minutes).padStart(2, "0")}:${secs}`
    : `${String(minutes)}:${secs}`;
}

/** a time moved onto the nearest frame start, so trims land on real frames */
export function snapToFrame(seconds: number, fps: number | null): number {
  if (!fps || fps <= 0) return Math.max(0, seconds);
  return Math.max(0, Math.round(seconds * fps) / fps);
}

/** the clip is never shorter than this, so there is always something to make */
export function clampTrim(
  start: number,
  end: number,
  duration: number,
  minimum = 0.1,
): { start: number; end: number } {
  const s = Math.min(Math.max(0, start), Math.max(0, duration - minimum));
  const e = Math.max(Math.min(duration, end), s + minimum);
  return { start: s, end: Math.min(duration, e) };
}

/** the frames a gif starts with in the editor: every frame, its own delay */
export function originalFrames(delaysMs: number[]): EditedFrame[] {
  return delaysMs.map((delayMs, source) => ({ source, delayMs }));
}

export function deleteFrames(frames: EditedFrame[], picked: Set<number>): EditedFrame[] {
  return frames.filter((_, i) => !picked.has(i));
}

/** each picked frame gets a copy straight after it */
export function duplicateFrames(frames: EditedFrame[], picked: Set<number>): EditedFrame[] {
  return frames.flatMap((frame, i) => (picked.has(i) ? [frame, { ...frame }] : [frame]));
}

/** moves the frame at `from` so it ends up at `to` */
export function moveFrame(frames: EditedFrame[], from: number, to: number): EditedFrame[] {
  if (from === to || from < 0 || from >= frames.length) return frames;
  const next = [...frames];
  const [moved] = next.splice(from, 1);
  if (!moved) return frames;
  next.splice(Math.max(0, Math.min(next.length, to)), 0, moved);
  return next;
}

export function setDelays(frames: EditedFrame[], picked: Set<number>, delayMs: number) {
  const delay = Math.max(10, Math.min(60_000, Math.round(delayMs / 10) * 10));
  return frames.map((frame, i) => (picked.has(i) ? { ...frame, delayMs: delay } : frame));
}

export function totalMs(frames: EditedFrame[]): number {
  return frames.reduce((sum, f) => sum + f.delayMs, 0);
}

/** the formats every one of `kinds` can become, in the order of the first */
export function commonTargets(kinds: MediaKind[]): ConvertTarget[] {
  const lists = kinds.map((kind) => toolDefaults.convertTargets[kind] ?? []);
  const [first, ...rest] = lists;
  if (!first) return [];
  return first.filter((t) => rest.every((list) => list.includes(t)));
}

export const targetInfo: Record<ConvertTarget, { label: string; hint: string }> = {
  mp4: { label: "MP4", hint: "Plays everywhere" },
  webm: { label: "WebM", hint: "Open format for the web" },
  mkv: { label: "MKV", hint: "Keeps every track" },
  mov: { label: "MOV", hint: "For Apple apps" },
  mp3: { label: "MP3", hint: "Sound, plays everywhere" },
  m4a: { label: "M4A", hint: "Sound, AAC" },
  ogg: { label: "OGG", hint: "Sound, Opus" },
  wav: { label: "WAV", hint: "Sound, uncompressed" },
  flac: { label: "FLAC", hint: "Sound, lossless" },
  jpg: { label: "JPG", hint: "Photos" },
  png: { label: "PNG", hint: "Lossless, keeps transparency" },
  webp: { label: "WebP", hint: "Small, keeps transparency" },
  avif: { label: "AVIF", hint: "Smallest, newer apps" },
  bmp: { label: "BMP", hint: "Uncompressed" },
  tiff: { label: "TIFF", hint: "For print and archives" },
  ico: { label: "ICO", hint: "Windows icon, many sizes" },
  gif: { label: "GIF", hint: "Moving picture" },
  frames: { label: "Frames", hint: "Every frame as a picture" },
};

export const videoTargets: ConvertTarget[] = ["mp4", "webm", "mkv", "mov"];
export const audioTargets: ConvertTarget[] = ["mp3", "m4a", "ogg", "wav", "flac"];
export const pictureTargets: ConvertTarget[] = ["jpg", "png", "webp", "avif", "bmp", "tiff", "ico"];

/** the codecs a container holds as they are, mirroring `ConvertTarget::copies` in rust */
const copies: Partial<Record<ConvertTarget, [string[], string[]]>> = {
  mp4: [
    ["h264", "hevc", "av1", "vp9", "mpeg4"],
    ["aac", "mp3", "opus", "ac3", "eac3", "alac", "flac"],
  ],
  mov: [
    ["h264", "hevc", "prores", "mjpeg", "mpeg4"],
    ["aac", "alac", "mp3", "pcm_s16le", "pcm_s24le"],
  ],
  webm: [
    ["vp8", "vp9", "av1"],
    ["opus", "vorbis"],
  ],
  mkv: [
    ["h264", "hevc", "av1", "vp8", "vp9", "mpeg4", "prores", "mjpeg", "ffv1"],
    [
      "aac",
      "mp3",
      "opus",
      "vorbis",
      "flac",
      "ac3",
      "eac3",
      "alac",
      "pcm_s16le",
      "pcm_s24le",
      "dts",
    ],
  ],
  mp3: [[], ["mp3"]],
  m4a: [[], ["aac", "alac"]],
  ogg: [[], ["opus", "vorbis"]],
  flac: [[], ["flac"]],
  wav: [[], ["pcm_s16le", "pcm_s24le", "pcm_f32le"]],
};

/** whether converting can copy the streams as they are, which is instant and lossless */
export function canRemux(info: MediaInfo, target: ConvertTarget, options: ConvertOptions) {
  const pair = copies[target];
  if (!pair) return false;
  const trimmed = (options.start ?? 0) > 0 || options.end !== null;
  const edits =
    options.maxWidth !== null ||
    options.maxHeight !== null ||
    options.rotate % 360 !== 0 ||
    options.flipHorizontal ||
    options.flipVertical ||
    Math.abs(options.speed - 1) > 1e-6;
  if (trimmed || edits) return false;
  const [videoOk, audioOk] = pair;
  const isVideo = videoTargets.includes(target);
  if (isVideo && !info.video) return false;
  if (isVideo && info.video && !videoOk.includes(info.video.codec)) return false;
  if (!isVideo && info.audio.length === 0) return false;
  return options.mute || info.audio.every((a) => audioOk.includes(a.codec));
}

/** a finished job's result, if it is a tool result */
export function toolResult(job: JobSnapshot): ToolResult | null {
  const result = job.result;
  if (!isPlainObject(result) || typeof result.path !== "string") return null;
  return result as unknown as ToolResult;
}

/** the jobs a session started, newest first */
export function sessionJobs(jobs: Record<number, JobSnapshot>, document: string): JobSnapshot[] {
  return Object.values(jobs)
    .filter((job) => job.owner === document)
    .sort((a, b) => b.id - a.id);
}

/** the latest job for each file of a session */
export function latestPerFile(jobs: JobSnapshot[]): Map<string, JobSnapshot> {
  const latest = new Map<string, JobSnapshot>();
  for (const job of jobs) {
    if (job.subject && !latest.has(job.subject)) latest.set(job.subject, job);
  }
  return latest;
}

/** "62% smaller", "4% bigger" */
export function savings(before: number, after: number): string | null {
  if (before <= 0) return null;
  const change = Math.round((1 - after / before) * 100);
  if (change === 0) return "about the same size";
  return change > 0 ? `${String(change)}% smaller` : `${String(-change)}% bigger`;
}

/** the renames a plan would make, leaving out files that keep their name or clash */
export function renamesFrom(plan: RenamePlan | null): { from: string; to: string }[] {
  if (!plan || plan.error) return [];
  return plan.rows
    .filter((row) => row.problem === null && row.from !== row.to)
    .map(({ from, to }) => ({ from, to }));
}
