// export to x mb: one size box with the platform presets beside it, a strategy, and the
// plan for each file before anything is encoded. the advanced drawer holds the codec,
// sound, trim and output formats.

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { SizeOptions } from "../../bindings/SizeOptions";
import type { SizeUnit } from "../../bindings/SizeUnit";
import type { TargetSize } from "../../bindings/TargetSize";
import { formatBytes } from "../../lib/format";
import { ipc } from "../../lib/ipc";
import { fileName, kindOf } from "../../lib/routing";
import { goalBytes, samePreset, sizeLabel } from "../../lib/tools";
import { useSettings } from "../../state/settings";
import { FilesColumn } from "./Files";
import {
  Advanced,
  Field,
  Note,
  NumberField,
  Section,
  Segmented,
  Toggle,
  type Choice,
} from "./parts";
import { useDebounced, useLatest, useSession, useTarget } from "./session";

const strategies: Choice<SizeOptions["strategy"]>[] = [
  {
    value: "bestQuality",
    label: "Best quality",
    tooltip: "Picks the size, frame rate and sound quality that look best at this size",
  },
  {
    value: "keepResolution",
    label: "Keep resolution",
    tooltip: "Never scales the picture down, lowers the frame rate instead",
  },
  {
    value: "keepFps",
    label: "Keep frame rate",
    tooltip: "Never drops frames, scales down instead",
  },
  {
    value: "keepAudio",
    label: "Keep sound",
    tooltip: "Keeps the sound at good quality and takes the room from the picture",
  },
];

const codecs: Choice<SizeOptions["codec"]>[] = [
  { value: "h264", label: "H.264", tooltip: "Plays everywhere, including inside Discord" },
  { value: "h265", label: "H.265", tooltip: "Smaller, but some apps and browsers cannot play it" },
  { value: "av1", label: "AV1", tooltip: "Smallest, free, needs newer apps" },
  { value: "vp9", label: "VP9", tooltip: "Free, plays in browsers, saved as WebM" },
];

export function ExportToSize({ doc }: { doc: DocumentInfo }) {
  const session = useSession(doc, "export_to_size");
  const { options, set, seal } = session;
  const exportSettings = useSettings((s) => s.settings?.export);
  const presets = exportSettings?.presets ?? [];
  const target = useTarget(options.target);
  const margin = options.marginPercent ?? exportSettings?.safetyMarginPercent ?? 3;
  const goal = goalBytes(target, margin);
  const single = doc.inputs.length === 1;
  const kinds = new Set(doc.inputs.map((p) => kindOf(p)?.kind));
  const hasVideo = kinds.has("video");
  const hasAudio = kinds.has("audio") || hasVideo;
  const hasImage = kinds.has("image") || kinds.has("raw") || kinds.has("vector");

  const setTarget = (next: TargetSize, preset: string | null) => {
    set({ target: next, preset });
  };

  return (
    <div className="grid min-h-0 flex-1 grid-rows-[1fr_auto] lg:grid-cols-[minmax(0,1fr)_24rem] lg:grid-rows-1">
      <div className="flex min-h-0 flex-col gap-6 overflow-y-auto p-5">
        <Section title="Target size">
          <div className="flex flex-wrap items-end gap-3">
            <label className="flex flex-col gap-1">
              <span className="sr-only">Target size</span>
              <input
                aria-label="Target size"
                type="number"
                min={0.01}
                step={0.5}
                value={Math.round(target.amount * 100) / 100}
                onChange={(event) => {
                  const amount = Number(event.target.value);
                  if (Number.isFinite(amount) && amount > 0) {
                    setTarget({ amount, unit: target.unit }, null);
                  }
                }}
                onBlur={seal}
                className="h-12 w-32 rounded-lg border border-line-strong bg-raised px-3 text-2xl font-semibold text-fg tabular focus:border-accent-line focus:outline-none"
              />
            </label>
            <Segmented<SizeUnit>
              label="Unit"
              value={target.unit}
              choices={[
                {
                  value: "MB",
                  label: "MB",
                  tooltip: "Megabytes of 1,000,000 bytes, as phones and Macs count",
                },
                {
                  value: "MiB",
                  label: "MiB",
                  tooltip: "Mebibytes of 1,048,576 bytes, as Windows counts",
                },
              ]}
              onChange={(unit) => {
                setTarget({ amount: target.amount, unit }, null);
              }}
            />
          </div>
          <div className="flex flex-wrap gap-1.5" role="group" aria-label="Presets">
            {presets.map((preset) => {
              const on =
                options.preset === preset.id ||
                (options.preset === null && options.target !== null && samePreset(preset, target));
              return (
                <button
                  key={preset.id}
                  type="button"
                  aria-pressed={on}
                  title={preset.note || `${preset.label}: ${sizeLabel(preset)}`}
                  onClick={() => {
                    setTarget({ amount: preset.amount, unit: preset.unit }, preset.id);
                  }}
                  className={`flex h-8 items-center gap-1.5 rounded-full border px-3 text-xs transition-colors duration-(--motion-fast) ${
                    on
                      ? "border-accent-line bg-accent-soft text-accent-text"
                      : "border-line bg-raised text-fg-muted hover:border-line-strong hover:text-fg"
                  }`}
                >
                  <span className="font-medium">{preset.label}</span>
                  <span className="text-fg-subtle tabular">{sizeLabel(preset)}</span>
                </button>
              );
            })}
          </div>
          <p className="text-xs text-fg-subtle tabular">
            Aims for {formatBytes(goal)}, {String(margin)}% under the limit, so files never land
            just over it.
          </p>
        </Section>

        {hasVideo && (
          <Section title="When something has to give">
            <Segmented
              label="Strategy"
              value={options.strategy}
              choices={strategies}
              onChange={(strategy) => {
                set({ strategy });
              }}
            />
          </Section>
        )}

        <Plans doc={doc} options={options} />

        <Advanced>
          {hasVideo && (
            <Field label="Video codec" hint="H.264 is the safe choice for chats and old phones.">
              <Segmented
                label="Video codec"
                value={options.codec}
                choices={codecs}
                onChange={(codec) => {
                  set({ codec });
                }}
              />
            </Field>
          )}
          {hasAudio && (
            <>
              {hasVideo && (
                <Toggle
                  label="Remove the sound"
                  hint="All the room goes to the picture."
                  checked={options.mute}
                  onChange={(mute) => {
                    set({ mute });
                  }}
                />
              )}
              <Toggle
                label="Mono sound"
                hint="Half the room for sound, fine for speech."
                checked={options.mono}
                onChange={(mono) => {
                  set({ mono });
                }}
              />
              <Field label="Sound format" hint="For songs and recordings.">
                <Segmented
                  label="Sound format"
                  value={options.audioFormat}
                  choices={[
                    { value: "mp3", label: "MP3", tooltip: "Plays everywhere" },
                    { value: "opus", label: "Opus", tooltip: "Sounds better at small sizes" },
                  ]}
                  onChange={(audioFormat) => {
                    set({ audioFormat });
                  }}
                />
              </Field>
            </>
          )}
          {hasImage && (
            <Field label="Picture format" hint="Auto keeps JPEG, PNG, WebP and AVIF as they are.">
              <Segmented
                label="Picture format"
                value={options.imageFormat}
                choices={[
                  { value: "auto", label: "Auto", tooltip: "Keeps the format where it can" },
                  { value: "jpeg", label: "JPEG", tooltip: "Photos, plays everywhere" },
                  { value: "webp", label: "WebP", tooltip: "Smaller, keeps transparency" },
                  { value: "avif", label: "AVIF", tooltip: "Smallest, newer apps" },
                  { value: "png", label: "PNG", tooltip: "Lossless, then fewer colours" },
                ]}
                onChange={(imageFormat) => {
                  set({ imageFormat });
                }}
              />
            </Field>
          )}
          {single && hasAudio && (
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
          <NumberField
            label="Safety margin"
            suffix="%"
            min={0}
            max={50}
            step={0.5}
            value={options.marginPercent}
            placeholder={String(exportSettings?.safetyMarginPercent ?? 3)}
            hint="Empty uses the margin from Settings."
            onChange={(marginPercent) => {
              set({ marginPercent });
            }}
          />
        </Advanced>
      </div>
      <FilesColumn
        doc={doc}
        jobs={session.jobs}
        onAdd={() => void session.addFiles()}
        onRemove={session.removeFile}
        runLabel={`Export to ${sizeLabel(target)}`}
        runTooltip="Shrink every file in the list under the size"
        onRun={() => void session.run()}
        canRun={!session.running}
      />
    </div>
  );
}

/** the plan for the first few files, worked out by rust as the options change */
function Plans({ doc, options }: { doc: DocumentInfo; options: SizeOptions }) {
  const settled = useDebounced(options, 300);
  const shown = doc.inputs.slice(0, 4);
  if (shown.length === 0) return null;
  return (
    <Section title="What will happen">
      <ul className="flex flex-col gap-2">
        {shown.map((path) => (
          <PlanRow key={path} path={path} options={settled} />
        ))}
      </ul>
      {doc.inputs.length > shown.length && (
        <p className="text-2xs text-fg-subtle">
          And {String(doc.inputs.length - shown.length)} more, each worked out the same way.
        </p>
      )}
    </Section>
  );
}

function PlanRow({ path, options }: { path: string; options: SizeOptions }) {
  const plan = useLatest(() => ipc.sizePlan(path, options), `${path}|${JSON.stringify(options)}`);
  const value = plan.value;
  return (
    <li className="flex flex-col gap-1">
      <span className="truncate text-xs font-medium" title={path}>
        {fileName(path)}
        {value && (
          <span className="font-normal text-fg-subtle">
            {" "}
            · now {formatBytes(value.sourceBytes)}
          </span>
        )}
      </span>
      {plan.error && <Note tone="danger">{plan.error}</Note>}
      {value?.problem && <Note tone="warning">{value.problem}</Note>}
      {value && !value.problem && (
        <Note tone={value.alreadyFits ? "success" : "muted"}>
          {value.summary}
          {value.video && value.video.changes.length > 0 && (
            <span className="mt-1 block text-fg-subtle">{value.video.changes.join(" ")}</span>
          )}
        </Note>
      )}
      {!value && !plan.error && plan.loading && <Note>Working it out...</Note>}
    </li>
  );
}
