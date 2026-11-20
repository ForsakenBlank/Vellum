// gif tools: make a gif smaller, edit it frame by frame, split it into pictures, or turn it
// into a video. the frame editor's list lives in the session's options, so every delete,
// duplicate, move and delay change can be undone.

import { convertFileSrc } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, Copy, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { EditedFrame } from "../../bindings/EditedFrame";
import type { GifFramesView } from "../../bindings/GifFramesView";
import type { GifMode } from "../../bindings/GifMode";
import { Button } from "../../components/Button";
import { Skeleton } from "../../components/Skeleton";
import { inDesktopApp, ipc } from "../../lib/ipc";
import {
  deleteFrames,
  duplicateFrames,
  moveFrame,
  originalFrames,
  setDelays,
  sizeLabel,
  toolDefaults,
  totalMs,
} from "../../lib/tools";
import { FilesColumn } from "./Files";
import { Field, Note, NumberField, Section, Segmented, Slider, Toggle, type Choice } from "./parts";
import { fail, useSession, useTarget } from "./session";

const modes: Choice<GifMode>[] = [
  {
    value: "optimise",
    label: "Make smaller",
    tooltip: "Fewer colours, lossy compression, narrower, fewer frames",
  },
  { value: "edit", label: "Edit frames", tooltip: "Delete, duplicate, reorder and retime frames" },
  { value: "split", label: "Split", tooltip: "Every frame as its own PNG in a folder" },
  { value: "video", label: "To video", tooltip: "An MP4 of the GIF, usually far smaller" },
];

const runLabels: Record<GifMode, string> = {
  optimise: "Make smaller",
  edit: "Save the edited GIF",
  split: "Split into frames",
  video: "Make an MP4",
};

export function GifTools({ doc }: { doc: DocumentInfo }) {
  const session = useSession(doc, "gif_tools");
  const { options, set, seal } = session;
  const target = useTarget(options.target);
  const first = doc.inputs[0] ?? null;

  return (
    <div className="grid min-h-0 flex-1 grid-rows-[1fr_auto] lg:grid-cols-[minmax(0,1fr)_24rem] lg:grid-rows-1">
      <div className="flex min-h-0 flex-col gap-6 overflow-y-auto p-5">
        <Segmented
          label="What to do"
          value={options.mode}
          choices={modes}
          onChange={(mode) => {
            set({ mode });
          }}
        />
        {options.mode === "optimise" && (
          <Section title="Make it smaller">
            <NumberField
              label="Width"
              suffix="px"
              min={16}
              value={options.width}
              placeholder="Same"
              hint="Empty keeps the width it has."
              onChange={(width) => {
                set({ width: width === null ? null : Math.round(width) });
              }}
            />
            <Slider
              label="Colours"
              min={16}
              max={256}
              step={16}
              value={options.colors}
              onChange={(colors) => {
                set({ colors });
              }}
              onCommit={seal}
            />
            <Slider
              label="Lossy compression"
              min={0}
              max={100}
              step={5}
              value={options.lossy}
              format={(v) => (v === 0 ? "Off" : String(v))}
              onChange={(lossy) => {
                set({ lossy });
              }}
              onCommit={seal}
            />
            <Toggle
              label="Drop every other frame"
              hint="Each frame stays up twice as long, so the timing is the same."
              checked={options.dropFrames}
              onChange={(dropFrames) => {
                set({ dropFrames });
              }}
            />
            <Toggle
              label={`Fit to ${sizeLabel(target)}`}
              hint="Starts from these settings and steps down until it fits."
              checked={options.fitToSize}
              onChange={(fitToSize) => {
                set({ fitToSize });
              }}
            />
          </Section>
        )}
        {options.mode === "edit" && first && (
          <FrameEditor
            path={first}
            frames={options.frames}
            loops={options.loops}
            onFrames={(frames) => {
              set({ frames });
            }}
            onLoops={(loops) => {
              set({ loops });
            }}
          />
        )}
        {options.mode === "edit" && doc.inputs.length > 1 && (
          <Note>The frame editor works on the first GIF in the list.</Note>
        )}
        {options.mode === "split" && (
          <Note>Every frame is saved as a numbered PNG in a new folder next to the GIF.</Note>
        )}
        {options.mode === "video" && (
          <Note>
            Makes an MP4 that plays like the GIF. It is usually a tenth of the size and looks
            better, and chats loop it the same way.
          </Note>
        )}
      </div>
      <FilesColumn
        doc={doc}
        jobs={session.jobs}
        onAdd={() => void session.addFiles()}
        onRemove={session.removeFile}
        runLabel={runLabels[options.mode]}
        runTooltip="Run it on the GIFs in the list"
        onRun={() => void session.run()}
        canRun={!session.running}
      />
    </div>
  );
}

function FrameEditor({
  path,
  frames,
  loops,
  onFrames,
  onLoops,
}: {
  path: string;
  frames: EditedFrame[] | null;
  loops: number | null;
  onFrames: (frames: EditedFrame[] | null) => void;
  onLoops: (loops: number | null) => void;
}) {
  const [view, setView] = useState<GifFramesView | null>(null);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [dragging, setDragging] = useState<number | null>(null);
  const [delay, setDelay] = useState<number | null>(null);

  useEffect(() => {
    if (!inDesktopApp) return;
    let stale = false;
    ipc
      .gifFrames(path)
      .then((next) => {
        if (stale) return;
        setView(next);
      })
      .catch(fail("The frames could not be read"));
    return () => {
      stale = true;
    };
  }, [path]);

  if (!view) {
    return <Skeleton className="h-48 w-full rounded-xl" />;
  }
  const list = frames ?? originalFrames(view.info.delaysMs);
  const change = (next: EditedFrame[]) => {
    onFrames(next);
    setPicked(new Set());
  };
  const tooFast = list.some((f) => f.delayMs < toolDefaults.minSafeDelayMs);
  const sheet = convertFileSrc(view.sheet);
  const pickedOne = picked.size === 1 ? [...picked][0] : undefined;

  return (
    <Section
      title={`${String(list.length)} frames · ${(totalMs(list) / 1000).toFixed(2)} s`}
      aside={
        <Button
          size="sm"
          variant="ghost"
          tooltip="Put every frame back the way the GIF had them"
          icon={<RotateCcw className="size-3.5" />}
          disabled={frames === null}
          onClick={() => {
            change(originalFrames(view.info.delaysMs));
            onFrames(null);
          }}
        >
          Start over
        </Button>
      }
    >
      <div className="flex flex-wrap items-end gap-2">
        <Button
          size="sm"
          tooltip="Delete the picked frames"
          icon={<Trash2 className="size-3.5" />}
          disabled={picked.size === 0 || picked.size === list.length}
          onClick={() => {
            change(deleteFrames(list, picked));
          }}
        >
          Delete
        </Button>
        <Button
          size="sm"
          tooltip="Copy each picked frame right after itself"
          icon={<Copy className="size-3.5" />}
          disabled={picked.size === 0}
          onClick={() => {
            change(duplicateFrames(list, picked));
          }}
        >
          Duplicate
        </Button>
        <Button
          size="sm"
          tooltip="Move the picked frame one place earlier"
          icon={<ArrowLeft className="size-3.5" />}
          disabled={pickedOne === undefined || pickedOne === 0}
          onClick={() => {
            if (pickedOne === undefined) return;
            onFrames(moveFrame(list, pickedOne, pickedOne - 1));
            setPicked(new Set([pickedOne - 1]));
          }}
        >
          Earlier
        </Button>
        <Button
          size="sm"
          tooltip="Move the picked frame one place later"
          icon={<ArrowRight className="size-3.5" />}
          disabled={pickedOne === undefined || pickedOne === list.length - 1}
          onClick={() => {
            if (pickedOne === undefined) return;
            onFrames(moveFrame(list, pickedOne, pickedOne + 1));
            setPicked(new Set([pickedOne + 1]));
          }}
        >
          Later
        </Button>
        <div className="ml-auto flex items-end gap-2">
          <NumberField
            label="Delay"
            suffix="ms"
            min={10}
            step={10}
            value={delay}
            placeholder={pickedOne !== undefined ? String(list[pickedOne]?.delayMs ?? "") : "100"}
            onChange={setDelay}
          />
          <Button
            size="sm"
            tooltip="Show the picked frames for this long"
            disabled={picked.size === 0 || delay === null}
            onClick={() => {
              if (delay !== null) change(setDelays(list, picked, delay));
            }}
          >
            Set
          </Button>
        </div>
      </div>
      {tooFast && (
        <Note tone="warning">
          Some frames are shorter than {toolDefaults.minSafeDelayMs} ms. Browsers show those at 100
          ms, so the GIF would play slower than you expect.
        </Note>
      )}
      <ol
        aria-label="Frames"
        className="grid grid-cols-[repeat(auto-fill,minmax(6.5rem,1fr))] gap-2"
      >
        {list.map((frame, index) => {
          const col = frame.source % view.columns;
          const row = Math.floor(frame.source / view.columns);
          const on = picked.has(index);
          return (
            <li
              key={`${String(index)}-${String(frame.source)}`}
              draggable
              onDragStart={() => {
                setDragging(index);
              }}
              onDragOver={(event) => {
                event.preventDefault();
              }}
              onDrop={() => {
                if (dragging !== null) onFrames(moveFrame(list, dragging, index));
                setDragging(null);
              }}
            >
              <button
                type="button"
                aria-pressed={on}
                aria-label={`Frame ${String(index + 1)}, ${String(frame.delayMs)} milliseconds`}
                onClick={(event) => {
                  const next = new Set(
                    event.shiftKey || event.ctrlKey || event.metaKey ? picked : [],
                  );
                  if (on && next.has(index)) next.delete(index);
                  else next.add(index);
                  setPicked(next);
                }}
                className={`flex w-full flex-col gap-1 rounded-md border p-1 text-left transition-colors ${
                  on
                    ? "border-accent bg-accent-soft"
                    : "border-line bg-raised hover:border-line-strong"
                }`}
              >
                <span
                  className="checker block w-full rounded-sm bg-no-repeat"
                  style={{
                    aspectRatio: `${String(view.frameWidth)} / ${String(view.frameHeight)}`,
                    backgroundImage: `url("${sheet}")`,
                    backgroundSize: `${String(view.columns * 100)}% auto`,
                    backgroundPosition: `${String(view.columns > 1 ? (col / (view.columns - 1)) * 100 : 0)}% ${String(
                      rowPercent(row, view),
                    )}%`,
                  }}
                />
                <span className="flex justify-between text-2xs text-fg-subtle tabular">
                  <span>{index + 1}</span>
                  <span>{frame.delayMs} ms</span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
      <Field label="Plays" hint="How many times it plays before stopping.">
        <Segmented<number>
          label="Plays"
          value={loops ?? view.info.loops}
          choices={[
            { value: 0, label: "Forever", tooltip: "Loops forever" },
            { value: 1, label: "Once", tooltip: "Plays once and stops" },
            { value: 3, label: "3 times", tooltip: "Plays three times" },
          ]}
          onChange={onLoops}
        />
      </Field>
    </Section>
  );
}

/** where a sheet row sits as a css background position */
function rowPercent(row: number, view: GifFramesView): number {
  const rows = Math.ceil(view.info.delaysMs.length / view.columns);
  return rows > 1 ? (row / (rows - 1)) * 100 : 0;
}
