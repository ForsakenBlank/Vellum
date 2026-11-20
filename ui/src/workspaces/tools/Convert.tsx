// the universal converter. it offers the formats every file in the batch can become, then
// only the options that make sense for that format. when the streams can simply be copied
// into the new container it says so: that is instant and loses nothing.

import { FlipHorizontal2, FlipVertical2, RotateCw, Zap } from "lucide-react";

import type { ConvertTarget } from "../../bindings/ConvertTarget";
import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { FileKind } from "../../bindings/FileKind";
import type { MediaKind } from "../../bindings/MediaKind";
import { Button } from "../../components/Button";
import { kindOf } from "../../lib/routing";
import {
  audioTargets,
  canRemux,
  commonTargets,
  pictureTargets,
  targetInfo,
  videoTargets,
} from "../../lib/tools";
import { useMediaInfo } from "../../state/media";
import { FilesColumn } from "./Files";
import { Field, Note, NumberField, Section, Segmented, Slider, Toggle } from "./parts";
import { useSession } from "./session";

/** what kind of media a file is, from its extension, close enough to pick formats */
const mediaKinds: Partial<Record<FileKind, MediaKind>> = {
  image: "image",
  raw: "image",
  vector: "vector",
  gif: "animation",
  video: "video",
  audio: "audio",
};

const icoSizes = [16, 24, 32, 48, 64, 128, 256];

export function Convert({ doc }: { doc: DocumentInfo }) {
  const session = useSession(doc, "convert");
  const { options, set, seal } = session;
  const kinds = [
    ...new Set(doc.inputs.map((p) => mediaKinds[kindOf(p)?.kind ?? "pdf"] ?? "unknown")),
  ];
  const targets = commonTargets(kinds);
  const target: ConvertTarget | null =
    options.target && targets.includes(options.target) ? options.target : (targets[0] ?? null);
  const first = doc.inputs[0] ?? null;
  const probe = useMediaInfo(first);
  const single = doc.inputs.length === 1;
  const allPictures = kinds.every((k) => k === "image" || k === "vector");
  const slideshow = allPictures && target !== null && videoTargets.includes(target);
  const isVideo = target !== null && videoTargets.includes(target) && !slideshow;
  const isAudio = target !== null && audioTargets.includes(target);
  const isPicture = target !== null && pictureTargets.includes(target);
  const remux =
    single && target && probe?.state === "ready" ? canRemux(probe.info, target, options) : false;
  const timed =
    (isVideo || isAudio || target === "gif" || target === "frames") &&
    kinds.some((k) => k === "video" || k === "audio" || k === "animation");

  return (
    <div className="grid min-h-0 flex-1 grid-rows-[1fr_auto] lg:grid-cols-[minmax(0,1fr)_24rem] lg:grid-rows-1">
      <div className="flex min-h-0 flex-col gap-6 overflow-y-auto p-5">
        <Section title="Convert to">
          {targets.length === 0 ? (
            <Note tone="warning">
              {doc.inputs.length === 0
                ? "Add files to see what they can become."
                : "These files have no format in common. Convert them in separate batches."}
            </Note>
          ) : (
            <div
              role="radiogroup"
              aria-label="Format"
              className="grid grid-cols-[repeat(auto-fill,minmax(7.5rem,1fr))] gap-2"
            >
              {targets.map((t) => {
                const on = t === target;
                return (
                  <button
                    key={t}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    title={targetInfo[t].hint}
                    onClick={() => {
                      set({ target: t });
                    }}
                    className={`flex flex-col items-start gap-0.5 rounded-lg border px-3 py-2 text-left transition-colors duration-(--motion-fast) ${
                      on
                        ? "border-accent-line bg-accent-soft"
                        : "border-line bg-raised hover:border-line-strong"
                    }`}
                  >
                    <span
                      className={`text-sm font-semibold ${on ? "text-accent-text" : "text-fg"}`}
                    >
                      {targetInfo[t].label}
                    </span>
                    <span className="text-2xs text-fg-subtle">{targetInfo[t].hint}</span>
                  </button>
                );
              })}
            </div>
          )}
          {remux && (
            <Note tone="success">
              <Zap className="mr-1 inline size-3.5 align-[-2px]" />
              Instant: the streams already suit {target ? targetInfo[target].label : ""}, so they
              are copied as they are and nothing is lost.
            </Note>
          )}
          {slideshow && (
            <Note>
              The pictures become one video, in the order of the list, letterboxed to the shape of
              the first.
            </Note>
          )}
        </Section>

        {target && (
          <Section title="Options">
            {(isVideo ||
              isAudio ||
              (isPicture && ["jpg", "webp", "avif"].includes(target)) ||
              target === "gif") && (
              <Slider
                label="Quality"
                min={10}
                max={100}
                step={5}
                value={options.quality}
                onChange={(quality) => {
                  set({ quality });
                }}
                onCommit={seal}
                hint={
                  isAudio
                    ? "Higher keeps more detail in the sound."
                    : "Higher looks better and makes bigger files."
                }
              />
            )}
            {isVideo && ["mp4", "mkv", "mov"].includes(target) && (
              <Field label="Video codec">
                <Segmented
                  label="Video codec"
                  value={options.codec}
                  choices={[
                    { value: "h264", label: "H.264", tooltip: "Plays everywhere" },
                    {
                      value: "h265",
                      label: "H.265",
                      tooltip: "Half the size, some apps cannot play it",
                    },
                    { value: "av1", label: "AV1", tooltip: "Smallest, needs newer apps" },
                  ]}
                  onChange={(codec) => {
                    set({ codec });
                  }}
                />
              </Field>
            )}
            {slideshow && (
              <NumberField
                label="Each picture shows for"
                suffix="seconds"
                min={0.1}
                step={0.5}
                value={options.slideSeconds}
                onChange={(slideSeconds) => {
                  if (slideSeconds !== null) set({ slideSeconds });
                }}
              />
            )}
            {(isVideo || isPicture || target === "gif" || target === "frames" || slideshow) &&
              target !== "ico" && (
                <div className="flex flex-wrap gap-4">
                  <NumberField
                    label={kinds.includes("vector") ? "Width" : "Largest width"}
                    suffix="px"
                    min={1}
                    value={options.maxWidth}
                    placeholder="Same"
                    hint={
                      kinds.includes("vector")
                        ? "Drawings are drawn at exactly this width."
                        : undefined
                    }
                    onChange={(maxWidth) => {
                      set({ maxWidth: maxWidth === null ? null : Math.round(maxWidth) });
                    }}
                  />
                  <NumberField
                    label="Largest height"
                    suffix="px"
                    min={1}
                    value={options.maxHeight}
                    placeholder="Same"
                    onChange={(maxHeight) => {
                      set({ maxHeight: maxHeight === null ? null : Math.round(maxHeight) });
                    }}
                  />
                </div>
              )}
            {(isVideo || isPicture || target === "gif" || target === "frames") && (
              <Field label="Turn and flip">
                <div className="flex flex-wrap gap-1.5">
                  <Button
                    size="sm"
                    tooltip="Turn a quarter clockwise"
                    icon={<RotateCw className="size-3.5" />}
                    variant={options.rotate % 360 !== 0 ? "primary" : "secondary"}
                    onClick={() => {
                      set({ rotate: (options.rotate + 90) % 360 });
                    }}
                  >
                    {options.rotate % 360 === 0 ? "Turn" : `${String(options.rotate % 360)}°`}
                  </Button>
                  <Button
                    size="sm"
                    tooltip="Mirror left to right"
                    icon={<FlipHorizontal2 className="size-3.5" />}
                    variant={options.flipHorizontal ? "primary" : "secondary"}
                    onClick={() => {
                      set({ flipHorizontal: !options.flipHorizontal });
                    }}
                  >
                    Mirror
                  </Button>
                  <Button
                    size="sm"
                    tooltip="Flip upside down"
                    icon={<FlipVertical2 className="size-3.5" />}
                    variant={options.flipVertical ? "primary" : "secondary"}
                    onClick={() => {
                      set({ flipVertical: !options.flipVertical });
                    }}
                  >
                    Flip
                  </Button>
                </div>
              </Field>
            )}
            {target === "ico" && (
              <Field
                label="Icon sizes"
                hint="Windows picks the best one for each place it shows the icon."
              >
                <div className="flex flex-wrap gap-1.5">
                  {icoSizes.map((size) => {
                    const on = options.icoSizes.includes(size);
                    return (
                      <Button
                        key={size}
                        size="sm"
                        tooltip={`${String(size)} by ${String(size)} pixels`}
                        variant={on ? "primary" : "secondary"}
                        aria-pressed={on}
                        onClick={() => {
                          const next = on
                            ? options.icoSizes.filter((s) => s !== size)
                            : [...options.icoSizes, size].sort((a, b) => a - b);
                          if (next.length > 0) set({ icoSizes: next });
                        }}
                      >
                        {size}
                      </Button>
                    );
                  })}
                </div>
              </Field>
            )}
            {target === "frames" && (
              <>
                <NumberField
                  label="One frame every"
                  suffix="seconds"
                  min={0}
                  step={0.5}
                  value={options.frameInterval === 0 ? null : options.frameInterval}
                  placeholder="Every frame"
                  hint="Empty saves every single frame."
                  onChange={(frameInterval) => {
                    set({ frameInterval: frameInterval ?? 0 });
                  }}
                />
                <Toggle
                  label="Save as JPG"
                  hint="Much smaller than PNG, fine for photos."
                  checked={options.framesAsJpg}
                  onChange={(framesAsJpg) => {
                    set({ framesAsJpg });
                  }}
                />
              </>
            )}
            {timed && (
              <>
                <Slider
                  label="Speed"
                  min={0.25}
                  max={4}
                  step={0.25}
                  value={options.speed}
                  format={(v) => `${String(v)}×`}
                  onChange={(speed) => {
                    set({ speed });
                  }}
                  onCommit={seal}
                />
                {single && (
                  <div className="flex flex-wrap gap-4">
                    <NumberField
                      label="Start at"
                      suffix="s"
                      min={0}
                      step={0.1}
                      value={options.start}
                      placeholder="0"
                      onChange={(start) => {
                        set({ start });
                      }}
                    />
                    <NumberField
                      label="End at"
                      suffix="s"
                      min={0}
                      step={0.1}
                      value={options.end}
                      placeholder="the end"
                      onChange={(end) => {
                        set({ end });
                      }}
                    />
                  </div>
                )}
              </>
            )}
            {isVideo && (
              <Toggle
                label="Remove the sound"
                checked={options.mute}
                onChange={(mute) => {
                  set({ mute });
                }}
              />
            )}
          </Section>
        )}
      </div>
      <FilesColumn
        doc={doc}
        jobs={session.jobs}
        onAdd={() => void session.addFiles()}
        onRemove={session.removeFile}
        runLabel={target ? `Convert to ${targetInfo[target].label}` : "Convert"}
        runTooltip="Convert every file in the list"
        onRun={() => {
          // the first format shown is the one used when the user has not picked one
          if (target && options.target !== target) set({ target });
          void session.run();
        }}
        canRun={!session.running && target !== null}
      />
    </div>
  );
}
