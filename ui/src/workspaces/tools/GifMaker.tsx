// video to gif: pick the moment on a filmstrip with handles that land on real frames, drag
// a crop on the picture, add a caption, and see roughly how big the result will be before
// making it. the same view makes animated webp, apng, avif and silent mp4 loops.

import { convertFileSrc } from "@tauri-apps/api/core";
import { Crop as CropIcon } from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { AnimEstimate } from "../../bindings/AnimEstimate";
import type { AnimFormat } from "../../bindings/AnimFormat";
import type { AnimOptions } from "../../bindings/AnimOptions";
import type { Crop } from "../../bindings/Crop";
import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { MediaInfo } from "../../bindings/MediaInfo";
import type { TargetSize } from "../../bindings/TargetSize";
import { Button } from "../../components/Button";
import { Skeleton } from "../../components/Skeleton";
import { formatBytes } from "../../lib/format";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { clampTrim, formatTime, sizeLabel, snapToFrame } from "../../lib/tools";
import { assetFile, useAsset, useMediaInfo } from "../../state/media";
import { FilesColumn } from "./Files";
import {
  Advanced,
  Field,
  Note,
  NumberField,
  Section,
  Segmented,
  Slider,
  TextField,
  Toggle,
  type Choice,
} from "./parts";
import { useDebounced, useLatest, useSession, useTarget, type Latest } from "./session";

const formats: Choice<AnimFormat>[] = [
  { value: "gif", label: "GIF", tooltip: "Plays everywhere, biggest files" },
  { value: "webp", label: "WebP", tooltip: "Animated WebP, often a third of the GIF size" },
  { value: "apng", label: "APNG", tooltip: "Animated PNG, full colour, big files" },
  { value: "avif", label: "AVIF", tooltip: "Smallest animated picture, newer apps only" },
  {
    value: "mp4",
    label: "MP4 loop",
    tooltip: "A silent video, far smaller, chats loop it like a GIF",
  },
];

const PREVIEW_HEIGHT = 360;

export function GifMaker({ doc }: { doc: DocumentInfo }) {
  const session = useSession(doc, "video_to_gif");
  const { options, set, seal } = session;
  const anim = options.anim;
  const first = doc.inputs[0] ?? null;
  const single = doc.inputs.length === 1;
  const probe = useMediaInfo(first);
  const info = probe?.state === "ready" ? probe.info : null;
  const target = useTarget(options.target);
  const setAnim = (patch: Partial<AnimOptions>) => {
    set({ anim: patch });
  };

  const settled = useDebounced(anim, 500);
  const estimate = useLatest(
    first && info ? () => ipc.animEstimate(first, settled) : null,
    `${first ?? ""}|${info?.path ?? ""}|${JSON.stringify(settled)}`,
  );

  return (
    <div className="grid min-h-0 flex-1 grid-rows-[1fr_auto] lg:grid-cols-[minmax(0,1fr)_24rem] lg:grid-rows-1">
      <div className="flex min-h-0 flex-col gap-6 overflow-y-auto p-5">
        {first && single && (
          <Moment path={first} info={info} anim={anim} setAnim={setAnim} seal={seal} />
        )}
        {!single && doc.inputs.length > 1 && (
          <Note>
            Every video in the batch is made whole with these settings. Open one video on its own to
            trim or crop it.
          </Note>
        )}

        <Section title="Size and speed">
          <Field label="Width">
            <div className="flex flex-wrap items-center gap-2">
              <Segmented<number>
                label="Width"
                value={anim.width ?? 0}
                choices={[
                  { value: 320, label: "320", tooltip: "Small, for reactions and stickers" },
                  { value: 480, label: "480", tooltip: "The usual size for chats" },
                  { value: 640, label: "640", tooltip: "Sharper, bigger file" },
                  { value: 0, label: "Full", tooltip: "Keep the video's own width" },
                ]}
                onChange={(width) => {
                  setAnim({ width: width === 0 ? null : width });
                }}
              />
            </div>
          </Field>
          <Slider
            label="Frames per second"
            min={5}
            max={30}
            value={anim.fps ?? 15}
            format={(v) => `${String(v)} fps`}
            onChange={(fps) => {
              setAnim({ fps });
            }}
            onCommit={seal}
            hint="15 looks smooth and keeps the file small."
          />
          <Slider
            label="Speed"
            min={0.25}
            max={4}
            step={0.25}
            value={anim.speed}
            format={(v) => `${String(v)}×`}
            onChange={(speed) => {
              setAnim({ speed });
            }}
            onCommit={seal}
          />
          <div className="flex flex-col gap-3">
            <Toggle
              label="Play backwards"
              checked={anim.reverse}
              onChange={(reverse) => {
                setAnim({ reverse, boomerang: reverse ? false : anim.boomerang });
              }}
            />
            <Toggle
              label="Boomerang"
              hint="Plays forward, then backward."
              checked={anim.boomerang}
              onChange={(boomerang) => {
                setAnim({ boomerang, reverse: boomerang ? false : anim.reverse });
              }}
            />
          </div>
        </Section>

        <Section title="Caption">
          <TextField
            label="Top text"
            value={anim.caption.top}
            placeholder="Nothing"
            onChange={(top) => {
              setAnim({ caption: { ...anim.caption, top } });
            }}
          />
          <TextField
            label="Bottom text"
            value={anim.caption.bottom}
            placeholder="Nothing"
            onChange={(bottom) => {
              setAnim({ caption: { ...anim.caption, bottom } });
            }}
          />
          <div className="flex flex-wrap items-end gap-4">
            <Field label="Style">
              <Segmented
                label="Caption style"
                value={anim.caption.style}
                choices={[
                  {
                    value: "classic",
                    label: "Classic meme",
                    tooltip: "White capitals with a black outline, on the picture",
                  },
                  {
                    value: "bar",
                    label: "Caption bar",
                    tooltip: "Black text on a white bar above the picture",
                  },
                ]}
                onChange={(style) => {
                  setAnim({ caption: { ...anim.caption, style } });
                }}
              />
            </Field>
          </div>
          <Slider
            label="Text size"
            min={0.5}
            max={2}
            step={0.1}
            value={anim.caption.scale || 1}
            format={(v) => `${String(Math.round(v * 100))}%`}
            onChange={(scale) => {
              setAnim({ caption: { ...anim.caption, scale } });
            }}
            onCommit={seal}
          />
        </Section>

        <Section title="Format">
          <Segmented
            label="Format"
            value={anim.format}
            choices={formats}
            onChange={(format) => {
              setAnim({ format });
            }}
          />
          <Toggle
            label={`Fit to ${sizeLabel(target)}`}
            hint="Steps the settings down until the file fits, using the size from Export to size."
            checked={options.fitToSize}
            onChange={(fitToSize) => {
              set({ fitToSize });
            }}
          />
          <Advanced label="Quality and colours">
            {anim.format === "gif" ? (
              <>
                <Slider
                  label="Colours"
                  min={16}
                  max={256}
                  step={16}
                  value={anim.colors}
                  onChange={(colors) => {
                    setAnim({ colors });
                  }}
                  onCommit={seal}
                />
                <Field label="Dithering" hint="How smooth gradients look with few colours.">
                  <Segmented
                    label="Dithering"
                    value={anim.dither}
                    choices={[
                      { value: "sierra", label: "Smooth", tooltip: "Soft gradients, the default" },
                      {
                        value: "floydSteinberg",
                        label: "Fine",
                        tooltip: "Fine grain, a little bigger",
                      },
                      {
                        value: "bayer",
                        label: "Pattern",
                        tooltip: "An ordered pattern that compresses well",
                      },
                      { value: "none", label: "None", tooltip: "Flat colour bands, smallest" },
                    ]}
                    onChange={(dither) => {
                      setAnim({ dither });
                    }}
                  />
                </Field>
                <Slider
                  label="Lossy compression"
                  min={0}
                  max={100}
                  step={5}
                  value={anim.lossy}
                  format={(v) => (v === 0 ? "Off" : String(v))}
                  onChange={(lossy) => {
                    setAnim({ lossy });
                  }}
                  onCommit={seal}
                  hint="Lets colours that barely change stay as they were. Much smaller, hard to see."
                />
              </>
            ) : (
              <Slider
                label="Quality"
                min={0}
                max={100}
                step={5}
                value={anim.quality}
                onChange={(quality) => {
                  setAnim({ quality });
                }}
                onCommit={seal}
              />
            )}
            <NumberField
              label="Plays"
              min={0}
              step={1}
              value={anim.loops === 0 ? null : anim.loops}
              placeholder="Forever"
              hint="Empty loops forever."
              onChange={(loops) => {
                setAnim({ loops: loops === null ? 0 : Math.max(1, Math.round(loops)) });
              }}
            />
          </Advanced>
        </Section>
      </div>
      <FilesColumn
        doc={doc}
        jobs={session.jobs}
        onAdd={() => void session.addFiles()}
        onRemove={session.removeFile}
        runLabel={`Make ${formats.find((f) => f.value === anim.format)?.label ?? "GIF"}`}
        runTooltip="Make it from every video in the list"
        onRun={() => void session.run()}
        canRun={!session.running}
        footer={
          <Estimate value={estimate} format={anim.format} fit={options.fitToSize ? target : null} />
        }
      />
    </div>
  );
}

function Estimate({
  value,
  format,
  fit,
}: {
  value: Latest<AnimEstimate>;
  format: AnimFormat;
  fit: TargetSize | null;
}) {
  if (value.error) return <Note tone="danger">{value.error}</Note>;
  const e = value.value;
  if (!e) return value.loading ? <Note>Estimating the size...</Note> : null;
  const big = format === "gif" && e.bytes > 15_000_000;
  return (
    <div className="flex flex-col gap-1.5" aria-live="polite">
      <p className="text-xs text-fg-muted tabular">
        About <span className="font-semibold text-fg">{formatBytes(e.bytes)}</span> · {e.width}×
        {e.height} · {e.frames} frames · {e.seconds.toFixed(1)} s
        {value.loading ? " · updating" : ""}
      </p>
      {fit && (
        <p className="text-2xs text-fg-subtle">Will step down until it fits {sizeLabel(fit)}.</p>
      )}
      {big && (
        <Note tone="warning">
          That is a big GIF. An MP4 loop or WebP would be a fraction of the size and look better.
        </Note>
      )}
    </div>
  );
}

/** the frame at the start point with the crop on it, and the trim bar under it */
function Moment({
  path,
  info,
  anim,
  setAnim,
  seal,
}: {
  path: string;
  info: MediaInfo | null;
  anim: AnimOptions;
  setAnim: (patch: Partial<AnimOptions>) => void;
  seal: () => void;
}) {
  const duration = info?.durationSecs ?? 0;
  const fps = info?.video?.frameRate ?? null;
  const start = anim.start ?? 0;
  const end = anim.end ?? duration;
  const [frame, setFrame] = useState<string | null>(null);
  const shownAt = useDebounced(start, 150);

  useEffect(() => {
    if (!inDesktopApp || !info) return;
    let stale = false;
    ipc
      .mediaFrame(path, shownAt, PREVIEW_HEIGHT)
      .then((file) => {
        if (!stale) setFrame(file);
      })
      .catch(() => {
        // the filmstrip still shows where the cut is
      });
    return () => {
      stale = true;
    };
  }, [path, shownAt, info]);

  if (!info) {
    return <Skeleton className="aspect-video w-full rounded-xl" />;
  }
  const length = (end - start) / (anim.speed || 1);
  return (
    <Section
      title="Moment"
      aside={
        <span className="text-xs text-fg-subtle tabular">
          {formatTime(start)} to {formatTime(end)} · {length.toFixed(2)} s
          {anim.boomerang ? " there and back" : ""}
        </span>
      }
    >
      <CropPreview
        frame={frame}
        info={info}
        crop={anim.crop}
        caption={anim.caption}
        onCrop={(crop) => {
          setAnim({ crop });
          seal();
        }}
      />
      {duration > 0 && (
        <TrimBar
          path={path}
          duration={duration}
          fps={fps}
          start={start}
          end={end}
          onChange={(next) => {
            setAnim({ start: next.start, end: next.end });
          }}
          onCommit={seal}
        />
      )}
    </Section>
  );
}

function CropPreview({
  frame,
  info,
  crop,
  caption,
  onCrop,
}: {
  frame: string | null;
  info: MediaInfo;
  crop: Crop | null;
  caption: AnimOptions["caption"];
  onCrop: (crop: Crop | null) => void;
}) {
  const width = info.width ?? 1;
  const height = info.height ?? 1;
  const box = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState<Crop | null>(null);
  const shown = draft ?? crop;
  const gesture = useRef<{ mode: "new" | "move"; x: number; y: number; from: Crop | null } | null>(
    null,
  );

  const toSource = (event: ReactPointerEvent) => {
    const rect = box.current?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    return {
      x: Math.min(width, Math.max(0, ((event.clientX - rect.left) / rect.width) * width)),
      y: Math.min(height, Math.max(0, ((event.clientY - rect.top) / rect.height) * height)),
    };
  };
  const even = (v: number) => Math.max(0, Math.round(v / 2) * 2);

  const onDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const p = toSource(event);
    const inside =
      shown &&
      p.x >= shown.x &&
      p.x <= shown.x + shown.width &&
      p.y >= shown.y &&
      p.y <= shown.y + shown.height;
    gesture.current = { mode: inside ? "move" : "new", x: p.x, y: p.y, from: shown };
  };
  const onMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (!g) return;
    const p = toSource(event);
    if (g.mode === "move" && g.from) {
      const x = Math.min(width - g.from.width, Math.max(0, g.from.x + p.x - g.x));
      const y = Math.min(height - g.from.height, Math.max(0, g.from.y + p.y - g.y));
      setDraft({ ...g.from, x: even(x), y: even(y) });
    } else {
      setDraft({
        x: even(Math.min(g.x, p.x)),
        y: even(Math.min(g.y, p.y)),
        width: even(Math.abs(p.x - g.x)),
        height: even(Math.abs(p.y - g.y)),
      });
    }
  };
  const onUp = () => {
    const g = gesture.current;
    gesture.current = null;
    if (!g) return;
    // a click without a drag is not a crop
    if (draft && draft.width >= 16 && draft.height >= 16) onCrop(draft);
    setDraft(null);
  };

  const pct = (v: number, of: number) => `${String((v / of) * 100)}%`;
  return (
    <div className="flex flex-col gap-2">
      <div className="checker relative mx-auto w-full max-w-3xl overflow-hidden rounded-xl border border-line bg-canvas">
        <div
          ref={box}
          role="application"
          aria-label="Drag on the picture to crop it"
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          className="relative w-full cursor-crosshair touch-none [container-type:inline-size]"
          style={{ aspectRatio: `${String(width)} / ${String(height)}` }}
        >
          {frame ? (
            <img
              src={convertFileSrc(frame)}
              alt="The frame at the start point"
              draggable={false}
              className="absolute inset-0 size-full object-contain"
            />
          ) : (
            <Skeleton className="absolute inset-0 rounded-none" />
          )}
          {shown && (
            <div
              className="absolute border-2 border-accent shadow-[0_0_0_9999px_rgb(0_0_0/0.55)]"
              style={{
                left: pct(shown.x, width),
                top: pct(shown.y, height),
                width: pct(shown.width, width),
                height: pct(shown.height, height),
              }}
            />
          )}
          <CaptionPreview caption={caption} area={shown} width={width} height={height} />
        </div>
      </div>
      <div className="flex items-center gap-2 text-xs text-fg-subtle">
        <CropIcon className="size-3.5" />
        {crop ? (
          <>
            <span className="tabular">
              Cropped to {crop.width}×{crop.height}
            </span>
            <Button
              size="sm"
              variant="ghost"
              tooltip="Use the whole picture again"
              onClick={() => {
                onCrop(null);
              }}
            >
              Clear crop
            </Button>
          </>
        ) : (
          <span>Drag on the picture to crop it.</span>
        )}
      </div>
    </div>
  );
}

/** a rough look at the caption, drawn by the browser. the real one is drawn by rust. */
function CaptionPreview({
  caption,
  area,
  width,
  height,
}: {
  caption: AnimOptions["caption"];
  area: Crop | null;
  width: number;
  height: number;
}) {
  if (!caption.top.trim() && !caption.bottom.trim()) return null;
  const a = area ?? { x: 0, y: 0, width, height };
  const style = {
    left: `${String((a.x / width) * 100)}%`,
    top: `${String((a.y / height) * 100)}%`,
    width: `${String((a.width / width) * 100)}%`,
    height: `${String((a.height / height) * 100)}%`,
  };
  const classic = caption.style === "classic";
  const text = classic
    ? "font-black uppercase text-white [paint-order:stroke] [-webkit-text-stroke:0.12em_black]"
    : "font-black text-black bg-white px-2 py-1";
  return (
    <div
      aria-hidden
      className="pointer-events-none absolute flex flex-col items-center justify-between py-[3%] text-center"
      style={{ ...style, fontSize: `clamp(10px, ${String(3.2 * (caption.scale || 1))}cqw, 64px)` }}
    >
      <span
        className={`max-w-[94%] leading-tight ${text} ${caption.top.trim() ? "" : "invisible"}`}
      >
        {caption.top || "."}
      </span>
      <span
        className={`max-w-[94%] leading-tight ${text} ${caption.bottom.trim() ? "" : "invisible"}`}
      >
        {caption.bottom || "."}
      </span>
    </div>
  );
}

/** the filmstrip with in and out handles that snap to frames. arrow keys move a handle one
 * frame, shift and arrow one second. */
function TrimBar({
  path,
  duration,
  fps,
  start,
  end,
  onChange,
  onCommit,
}: {
  path: string;
  duration: number;
  fps: number | null;
  start: number;
  end: number;
  onChange: (next: { start: number; end: number }) => void;
  onCommit: () => void;
}) {
  const strip = assetFile(useAsset("filmstrip", path));
  const bar = useRef<HTMLDivElement>(null);
  const step = fps ? 1 / fps : 0.04;

  const move = (which: "start" | "end", seconds: number) => {
    const t = snapToFrame(seconds, fps);
    const next =
      which === "start" ? clampTrim(t, end, duration, step) : clampTrim(start, t, duration, step);
    onChange(next);
  };
  const timeAt = (clientX: number) => {
    const rect = bar.current?.getBoundingClientRect();
    if (!rect) return 0;
    return ((clientX - rect.left) / rect.width) * duration;
  };
  const handle = (which: "start" | "end") => {
    const value = which === "start" ? start : end;
    return (
      <div
        role="slider"
        tabIndex={0}
        aria-label={which === "start" ? "Start of the clip" : "End of the clip"}
        aria-valuemin={0}
        aria-valuemax={duration}
        aria-valuenow={value}
        aria-valuetext={formatTime(value)}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          event.stopPropagation();
        }}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            move(which, timeAt(event.clientX));
        }}
        onPointerUp={onCommit}
        onKeyDown={(event) => {
          const by = event.shiftKey ? 1 : step;
          if (event.key === "ArrowLeft") move(which, value - by);
          else if (event.key === "ArrowRight") move(which, value + by);
          else return;
          event.preventDefault();
        }}
        onKeyUp={onCommit}
        className="absolute inset-y-0 z-10 flex w-3 -translate-x-1/2 cursor-ew-resize items-center justify-center rounded-sm bg-accent shadow-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        style={{ left: `${String((value / duration) * 100)}%` }}
      >
        <span className="h-5 w-0.5 rounded-full bg-accent-fg/70" />
      </div>
    );
  };
  return (
    <div className="flex flex-col gap-1.5">
      <div
        ref={bar}
        className="relative h-14 overflow-visible rounded-md border border-line bg-raised"
      >
        {strip && (
          <img
            src={convertFileSrc(strip)}
            alt=""
            draggable={false}
            className="absolute inset-0 size-full rounded-md object-fill opacity-90"
          />
        )}
        <div
          className="absolute inset-y-0 left-0 rounded-l-md bg-canvas/70"
          style={{ width: `${String((start / duration) * 100)}%` }}
        />
        <div
          className="absolute inset-y-0 right-0 rounded-r-md bg-canvas/70"
          style={{ width: `${String((1 - end / duration) * 100)}%` }}
        />
        <div
          className="absolute inset-y-0 border-y-2 border-accent"
          style={{
            left: `${String((start / duration) * 100)}%`,
            right: `${String((1 - end / duration) * 100)}%`,
          }}
        />
        {handle("start")}
        {handle("end")}
      </div>
      <div className="flex justify-between text-2xs text-fg-subtle tabular">
        <span>0:00.00</span>
        <span>
          {fps
            ? `Handles land on frames, ${fps.toFixed(fps % 1 === 0 ? 0 : 2)} a second`
            : "Drag the handles"}
        </span>
        <span>{formatTime(duration)}</span>
      </div>
    </div>
  );
}
