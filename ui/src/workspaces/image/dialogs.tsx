import { useState } from "react";

import type { Depth } from "../../bindings/Depth";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ipc } from "../../lib/ipc";
import { windowLabel } from "../../lib/window";
import { toHex } from "./geometry";
import { useImageTools } from "./store";

export type ModifyOp = "feather" | "expand" | "contract" | "border" | "smooth";

const MODIFY: Record<ModifyOp, { title: string; label: string; hint: string; initial: number }> = {
  feather: {
    title: "Feather selection",
    label: "Feather radius",
    hint: "Softens the edge over this many pixels.",
    initial: 5,
  },
  expand: {
    title: "Expand selection",
    label: "Expand by",
    hint: "Grows the selection outwards, with rounded corners.",
    initial: 5,
  },
  contract: {
    title: "Contract selection",
    label: "Contract by",
    hint: "Shrinks the selection inwards.",
    initial: 5,
  },
  border: {
    title: "Border selection",
    label: "Width",
    hint: "Selects a band this wide along the edge.",
    initial: 10,
  },
  smooth: {
    title: "Smooth selection",
    label: "Radius",
    hint: "Rounds off corners and removes specks.",
    initial: 4,
  },
};

function NumberRow({
  id,
  label,
  value,
  unit,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  unit: string;
  onChange: (v: number) => void;
}) {
  return (
    <label htmlFor={id} className="flex items-center justify-between gap-3 text-sm">
      {label}
      <span className="flex items-center gap-1.5">
        <input
          id={id}
          type="number"
          min={0}
          value={value}
          onChange={(e) => {
            onChange(Math.max(0, Number(e.target.value) || 0));
          }}
          className="h-(--row) w-24 rounded-md border border-line bg-raised px-2 text-right tabular-nums"
        />
        <span className="w-6 text-xs text-fg-subtle">{unit}</span>
      </span>
    </label>
  );
}

export function ModifyDialog({
  doc,
  op,
  onClose,
  fail,
}: {
  doc: string;
  op: ModifyOp | null;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  // the parent keys this by `op`, so a new operation starts from its own default
  const info = op ? MODIFY[op] : null;
  const [amount, setAmount] = useState(info?.initial ?? 5);
  if (!op || !info) return null;
  const apply = () => {
    onClose();
    ipc.imageCommand(doc, "image.select.modify", { op, amount }).catch(fail);
  };
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={info.title}
      description={info.hint}
      width="24rem"
      footer={
        <>
          <Button tooltip="Close without changing the selection" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button tooltip="Change the selection" variant="primary" onClick={apply}>
            Apply
          </Button>
        </>
      }
    >
      <form
        className="px-5 py-4"
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <NumberRow
          id="modify-amount"
          label={info.label}
          value={amount}
          unit="px"
          onChange={setAmount}
        />
      </form>
    </Dialog>
  );
}

export function ColorRangeDialog({
  doc,
  open,
  onClose,
  fail,
}: {
  doc: string;
  open: boolean;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  const t = useImageTools();
  const [fuzziness, setFuzziness] = useState(40);
  const [invert, setInvert] = useState(false);
  if (!open) return null;
  const apply = () => {
    onClose();
    ipc
      .imageCommand(doc, "image.select.range", {
        colors: [t.foreground],
        fuzziness,
        invert,
        sampleAll: t.sampleAll,
      })
      .catch(fail);
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Colour range"
      description="Selects every pixel close to the foreground colour. Pick the colour with the eyedropper first."
      width="26rem"
      footer={
        <>
          <Button tooltip="Close without selecting" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button tooltip="Select the colour range" variant="primary" onClick={apply}>
            Select
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 px-5 py-4">
        <div className="flex items-center gap-2 text-sm">
          <span
            className="size-6 rounded-md border border-line-strong"
            style={{ background: toHex(t.foreground) }}
          />
          {toHex(t.foreground)}
        </div>
        <label className="flex items-center justify-between gap-3 text-sm">
          Fuzziness
          <input
            type="range"
            min={0}
            max={200}
            value={fuzziness}
            onChange={(e) => {
              setFuzziness(Number(e.target.value));
            }}
            className="flex-1 accent-(--accent)"
          />
          <span className="w-8 text-right tabular-nums">{fuzziness}</span>
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={invert}
            onChange={(e) => {
              setInvert(e.target.checked);
            }}
            className="accent-(--accent)"
          />
          Select everything else instead
        </label>
      </div>
    </Dialog>
  );
}

const PRESETS: [string, number, number][] = [
  ["Full HD", 1920, 1080],
  ["4K", 3840, 2160],
  ["Square post", 1080, 1080],
  ["Story", 1080, 1920],
  ["YouTube thumbnail", 1280, 720],
  ["A4 at 300 ppi", 2480, 3508],
];

type Background = "white" | "black" | "transparent" | "foreground";

export function NewImageDialog({
  open,
  onClose,
  fail,
}: {
  open: boolean;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  const fg = useImageTools((s) => s.foreground);
  const [width, setWidth] = useState(1920);
  const [height, setHeight] = useState(1080);
  const [depth, setDepth] = useState<Depth>("u8");
  const [background, setBackground] = useState<Background>("white");
  const [title, setTitle] = useState("");
  if (!open) return null;
  const create = () => {
    onClose();
    const bg: [number, number, number, number] | null =
      background === "white"
        ? [1, 1, 1, 1]
        : background === "black"
          ? [0, 0, 0, 1]
          : background === "foreground"
            ? [fg[0], fg[1], fg[2], 1]
            : null;
    ipc
      .imageNew(
        {
          title: title.trim() || null,
          width: Math.max(1, width),
          height: Math.max(1, height),
          depth,
          background: bg,
        },
        windowLabel,
      )
      .catch(fail);
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="New image"
      width="30rem"
      footer={
        <>
          <Button tooltip="Close without making a picture" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button tooltip="Make the new picture" variant="primary" onClick={create}>
            Create
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3 px-5 py-4"
        onSubmit={(e) => {
          e.preventDefault();
          create();
        }}
      >
        <label className="flex items-center justify-between gap-3 text-sm">
          Name
          <input
            value={title}
            placeholder="Untitled"
            onChange={(e) => {
              setTitle(e.target.value);
            }}
            className="h-(--row) w-56 rounded-md border border-line bg-raised px-2"
          />
        </label>
        <div className="flex flex-wrap gap-1">
          {PRESETS.map(([name, w, h]) => (
            <button
              key={name}
              type="button"
              aria-pressed={w === width && h === height}
              onClick={() => {
                setWidth(w);
                setHeight(h);
              }}
              className={`rounded-md border px-2 py-1 text-xs ${w === width && h === height ? "border-accent bg-accent-soft text-accent-text" : "border-line text-fg-muted hover:bg-elevated"}`}
            >
              {name}
            </button>
          ))}
        </div>
        <NumberRow id="new-width" label="Width" value={width} unit="px" onChange={setWidth} />
        <NumberRow id="new-height" label="Height" value={height} unit="px" onChange={setHeight} />
        <label className="flex items-center justify-between gap-3 text-sm">
          Bits per channel
          <select
            value={depth}
            onChange={(e) => {
              setDepth(e.target.value as Depth);
            }}
            className="h-(--row) w-56 rounded-md border border-line bg-raised px-2"
          >
            <option value="u8">8 bit, for most pictures</option>
            <option value="u16">16 bit, smoother gradients</option>
            <option value="f32">32 bit float, for HDR</option>
          </select>
        </label>
        <label className="flex items-center justify-between gap-3 text-sm">
          Background
          <select
            value={background}
            onChange={(e) => {
              setBackground(e.target.value as Background);
            }}
            className="h-(--row) w-56 rounded-md border border-line bg-raised px-2"
          >
            <option value="white">White</option>
            <option value="black">Black</option>
            <option value="foreground">Foreground colour</option>
            <option value="transparent">See through</option>
          </select>
        </label>
      </form>
    </Dialog>
  );
}
