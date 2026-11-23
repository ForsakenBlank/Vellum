import { save } from "@tauri-apps/plugin-dialog";
import { Link2, Link2Off } from "lucide-react";
import { useState } from "react";

import type { ExportFormat } from "../../bindings/ExportFormat";
import type { ExportOptions } from "../../bindings/ExportOptions";
import type { ImageState } from "../../bindings/ImageState";
import type { Interpolation } from "../../bindings/Interpolation";
import type { SizeUnit } from "../../bindings/SizeUnit";
import { Button, IconButton } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Tooltip } from "../../components/Tooltip";
import { ipc } from "../../lib/ipc";
import { useSettings } from "../../state/settings";
import { useUi } from "../../state/ui";
import { type ViewAids, useAids } from "./session";
import { keepRatio } from "./geometry";
import { useImageTools } from "./store";

// canvas size, image size, export and grid settings

const field = "h-(--row) rounded-md border border-line bg-raised px-2 text-right tabular-nums";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-3 text-sm">
      {label}
      <span className="flex items-center gap-1.5">{children}</span>
    </label>
  );
}

function Num({
  value,
  onChange,
  min = 1,
  max = 300000,
  label,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  label: string;
}) {
  return (
    <input
      type="number"
      aria-label={label}
      min={min}
      max={max}
      value={Number.isFinite(value) ? Number(value.toFixed(2)) : 0}
      onChange={(e) => {
        const v = Number(e.target.value);
        if (Number.isFinite(v)) onChange(v);
      }}
      className={`${field} w-24`}
    />
  );
}

/** where the picture stays when the canvas grows or shrinks, as fractions */
const ANCHORS: [number, number][] = [
  [0, 0],
  [0.5, 0],
  [1, 0],
  [0, 0.5],
  [0.5, 0.5],
  [1, 0.5],
  [0, 1],
  [0.5, 1],
  [1, 1],
];

const ANCHOR_NAMES = [
  "Top left",
  "Top",
  "Top right",
  "Left",
  "Middle",
  "Right",
  "Bottom left",
  "Bottom",
  "Bottom right",
];

type Extend = "transparent" | "white" | "black" | "background" | "foreground";

export function CanvasSizeDialog({
  doc,
  state,
  open,
  onClose,
  fail,
}: {
  doc: string;
  state: ImageState;
  open: boolean;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  const tools = useImageTools();
  const [width, setWidth] = useState(state.width);
  const [height, setHeight] = useState(state.height);
  const [relative, setRelative] = useState(false);
  const [anchor, setAnchor] = useState(4);
  const [extend, setExtend] = useState<Extend>("transparent");
  if (!open) return null;
  const w = relative ? state.width + width : width;
  const h = relative ? state.height + height : height;
  const apply = () => {
    onClose();
    const c =
      extend === "white"
        ? [1, 1, 1]
        : extend === "black"
          ? [0, 0, 0]
          : extend === "background"
            ? tools.background
            : extend === "foreground"
              ? tools.foreground
              : null;
    ipc
      .imageCommand(doc, "image.canvas.size", {
        width: Math.round(w),
        height: Math.round(h),
        anchor: ANCHORS[anchor] ?? [0.5, 0.5],
        fill: c ? [c[0], c[1], c[2], 1] : null,
      })
      .catch(fail);
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Canvas size"
      description="Adds or takes away canvas around the picture without scaling it. Pixels outside the canvas are kept and come back if it grows again."
      width="28rem"
      footer={
        <>
          <Button tooltip="Close without changing the canvas" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button tooltip="Change the canvas size" variant="primary" onClick={apply}>
            Apply
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3 px-5 py-4"
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <p className="text-xs text-fg-subtle">
          {`Now ${String(state.width)} × ${String(state.height)} px, after ${String(Math.round(w))} × ${String(Math.round(h))} px`}
        </p>
        <Row label="Width">
          <Num
            label="Width"
            value={width}
            min={relative ? -state.width + 1 : 1}
            onChange={setWidth}
          />
          <span className="w-6 text-xs text-fg-subtle">px</span>
        </Row>
        <Row label="Height">
          <Num
            label="Height"
            value={height}
            min={relative ? -state.height + 1 : 1}
            onChange={setHeight}
          />
          <span className="w-6 text-xs text-fg-subtle">px</span>
        </Row>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={relative}
            onChange={(e) => {
              setRelative(e.target.checked);
              setWidth(e.target.checked ? 0 : state.width);
              setHeight(e.target.checked ? 0 : state.height);
            }}
            className="accent-(--accent)"
          />
          Relative: add or take away this much
        </label>
        <div className="flex items-start justify-between gap-3 text-sm">
          Anchor
          <div role="radiogroup" aria-label="Anchor" className="grid grid-cols-3 gap-1">
            {ANCHORS.map((_, i) => (
              <Tooltip
                key={ANCHOR_NAMES[i]}
                text={`Keep the picture at the ${(ANCHOR_NAMES[i] ?? "").toLowerCase()}`}
              >
                <button
                  type="button"
                  role="radio"
                  aria-checked={anchor === i}
                  aria-label={ANCHOR_NAMES[i]}
                  onClick={() => {
                    setAnchor(i);
                  }}
                  className={`size-7 rounded-md border ${anchor === i ? "border-accent bg-accent-soft" : "border-line hover:bg-elevated"}`}
                />
              </Tooltip>
            ))}
          </div>
        </div>
        <Row label="New canvas">
          <select
            value={extend}
            onChange={(e) => {
              setExtend(e.target.value as Extend);
            }}
            className="h-(--row) w-48 rounded-md border border-line bg-raised px-2"
          >
            <option value="transparent">See through</option>
            <option value="white">White</option>
            <option value="black">Black</option>
            <option value="background">Background colour</option>
            <option value="foreground">Foreground colour</option>
          </select>
        </Row>
      </form>
    </Dialog>
  );
}

const RESAMPLE: [Interpolation, string][] = [
  ["bicubic", "Bicubic, smooth and sharp"],
  ["bilinear", "Bilinear, softer"],
  ["nearest", "Nearest neighbour, hard edges"],
];

export function ImageSizeDialog({
  doc,
  state,
  open,
  onClose,
  fail,
}: {
  doc: string;
  state: ImageState;
  open: boolean;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  const [width, setWidth] = useState(state.width);
  const [height, setHeight] = useState(state.height);
  const [unit, setUnit] = useState<"px" | "%">("px");
  const [linked, setLinked] = useState(true);
  const [resample, setResample] = useState<Interpolation>("bicubic");
  const [resolution, setResolution] = useState(state.resolution);
  if (!open) return null;
  const toPx = (v: number, base: number) => (unit === "%" ? Math.round((v / 100) * base) : v);
  const shown = (px: number, base: number) => (unit === "%" ? (px / base) * 100 : px);
  const apply = () => {
    onClose();
    ipc
      .imageCommand(doc, "image.image.size", {
        width: Math.max(1, Math.round(width)),
        height: Math.max(1, Math.round(height)),
        interpolation: resample,
        resolution: resolution !== state.resolution ? resolution : null,
      })
      .catch(fail);
  };
  const megapixels = (width * height) / 1e6;
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Image size"
      description="Scales the whole picture, every layer and mask with it."
      width="28rem"
      footer={
        <>
          <Button tooltip="Close without scaling" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button tooltip="Scale the picture" variant="primary" onClick={apply}>
            Apply
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3 px-5 py-4"
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <p className="text-xs text-fg-subtle">
          {`Now ${String(state.width)} × ${String(state.height)} px, after ${String(Math.round(width))} × ${String(Math.round(height))} px (${megapixels.toFixed(1)} megapixels)`}
        </p>
        <div className="flex items-center gap-2">
          <div className="flex flex-1 flex-col gap-3">
            <Row label="Width">
              <Num
                label="Width"
                value={shown(width, state.width)}
                min={unit === "%" ? 0.01 : 1}
                onChange={(v) => {
                  const px = Math.max(1, toPx(v, state.width));
                  setWidth(px);
                  if (linked) setHeight(keepRatio(px, state.width, state.height));
                }}
              />
            </Row>
            <Row label="Height">
              <Num
                label="Height"
                value={shown(height, state.height)}
                min={unit === "%" ? 0.01 : 1}
                onChange={(v) => {
                  const px = Math.max(1, toPx(v, state.height));
                  setHeight(px);
                  if (linked) setWidth(keepRatio(px, state.height, state.width));
                }}
              />
            </Row>
          </div>
          <IconButton
            label={linked ? "Keep proportions (on)" : "Keep proportions (off)"}
            active={linked}
            icon={linked ? <Link2 className="size-4" /> : <Link2Off className="size-4" />}
            onClick={() => {
              setLinked((l) => !l);
            }}
          />
          <select
            aria-label="Unit"
            value={unit}
            onChange={(e) => {
              setUnit(e.target.value as "px" | "%");
            }}
            className="h-(--row) rounded-md border border-line bg-raised px-1"
          >
            <option value="px">px</option>
            <option value="%">%</option>
          </select>
        </div>
        <Row label="Resolution">
          <Num label="Resolution" value={resolution} onChange={setResolution} />
          <span className="w-6 text-xs text-fg-subtle">ppi</span>
        </Row>
        <Row label="Resample">
          <select
            value={resample}
            onChange={(e) => {
              setResample(e.target.value as Interpolation);
            }}
            className="h-(--row) w-56 rounded-md border border-line bg-raised px-2"
          >
            {RESAMPLE.map(([id, text]) => (
              <option key={id} value={id}>
                {text}
              </option>
            ))}
          </select>
        </Row>
      </form>
    </Dialog>
  );
}

const FORMATS: [ExportFormat, string, string][] = [
  ["png", "PNG", "Lossless, keeps see through pixels. 16 bit pictures stay 16 bit."],
  ["jpeg", "JPEG", "Small photos that open anywhere. No see through pixels."],
  ["webp", "WebP", "Smaller than JPEG, keeps see through pixels."],
  ["avif", "AVIF", "The smallest, for the web. Saved in sRGB."],
];

const EXT: Record<ExportFormat, string> = { png: "png", jpeg: "jpg", webp: "webp", avif: "avif" };

const EXPORT_KEY = "vellum.imageExport";

function lastExport(): ExportOptions {
  const base: ExportOptions = {
    format: "png",
    quality: 90,
    target: null,
    background: [1, 1, 1],
    embedProfile: true,
  };
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(EXPORT_KEY) ?? "null");
    if (raw && typeof raw === "object") return { ...base, ...(raw as Partial<ExportOptions>) };
  } catch {
    // only a convenience
  }
  return base;
}

export function ExportDialog({
  doc,
  title,
  state,
  open,
  onClose,
  fail,
}: {
  doc: string;
  title: string;
  state: ImageState;
  open: boolean;
  onClose: () => void;
  fail: (err: unknown) => void;
}) {
  const settings = useSettings((s) => s.settings?.export);
  const [options, setOptions] = useState<ExportOptions>(lastExport);
  const [fit, setFit] = useState(options.target !== null);
  const [amount, setAmount] = useState(options.target?.amount ?? settings?.targetAmount ?? 10);
  const [unit, setUnit] = useState<SizeUnit>(options.target?.unit ?? settings?.unit ?? "MB");
  if (!open) return null;
  const patch = (p: Partial<ExportOptions>) => {
    setOptions((o) => ({ ...o, ...p }));
  };
  const lossy = options.format !== "png";
  const flat = options.format === "jpeg" || options.format === "avif";
  const go = async () => {
    const final: ExportOptions = { ...options, target: fit ? { amount, unit } : null };
    try {
      localStorage.setItem(EXPORT_KEY, JSON.stringify(final));
    } catch {
      // only a convenience
    }
    const ext = EXT[final.format];
    const stem = title.replace(/\.[^.]+$/, "") || "Untitled";
    const path = await save({
      title: `Export ${title}`,
      defaultPath: `${stem}.${ext}`,
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!path) return;
    onClose();
    await ipc.imageExport(doc, path, final);
    useUi.getState().toast({
      level: "info",
      title: "Exporting",
      body: "The picture is being saved. It shows in the jobs panel when it is done.",
    });
  };
  const presets = settings?.presets ?? [];
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Export as"
      width="30rem"
      footer={
        <>
          <Button tooltip="Close without exporting" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            tooltip="Pick where to save it"
            variant="primary"
            onClick={() => {
              go().catch(fail);
            }}
          >
            Export
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 px-5 py-4">
        <div role="radiogroup" aria-label="Format" className="grid grid-cols-4 gap-1">
          {FORMATS.map(([id, name, hint]) => (
            <Tooltip key={id} text={hint}>
              <button
                type="button"
                role="radio"
                aria-checked={options.format === id}
                onClick={() => {
                  patch({ format: id });
                }}
                className={`rounded-md border px-2 py-1.5 text-sm ${options.format === id ? "border-accent bg-accent-soft text-accent-text" : "border-line text-fg-muted hover:bg-elevated"}`}
              >
                {name}
              </button>
            </Tooltip>
          ))}
        </div>
        <p className="text-xs text-fg-subtle">
          {FORMATS.find((f) => f[0] === options.format)?.[2]}
          {options.format === "png" && state.depth !== "u8" && !fit
            ? " This picture will be saved at 16 bits."
            : ""}
        </p>
        {lossy && !fit && (
          <Row label="Quality">
            <input
              type="range"
              min={1}
              max={100}
              value={options.quality}
              aria-label="Quality"
              onChange={(e) => {
                patch({ quality: Number(e.target.value) });
              }}
              className="w-40 accent-(--accent)"
            />
            <span className="w-8 text-right text-xs tabular-nums">{options.quality}</span>
          </Row>
        )}
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={fit}
            onChange={(e) => {
              setFit(e.target.checked);
            }}
            className="accent-(--accent)"
          />
          Fit under a size
        </label>
        {fit && (
          <div className="flex flex-col gap-2 pl-6">
            <div className="flex items-center gap-1.5">
              <Num label="Size" value={amount} min={0.01} onChange={setAmount} />
              <select
                aria-label="Size unit"
                value={unit}
                onChange={(e) => {
                  setUnit(e.target.value as SizeUnit);
                }}
                className="h-(--row) rounded-md border border-line bg-raised px-1"
              >
                <option value="MB">MB</option>
                <option value="MiB">MiB</option>
              </select>
            </div>
            <div className="flex flex-wrap gap-1">
              {presets.map((p) => (
                <Tooltip key={p.id} text={p.note || `${String(p.amount)} ${p.unit}`}>
                  <button
                    type="button"
                    onClick={() => {
                      setAmount(p.amount);
                      setUnit(p.unit);
                    }}
                    className="rounded-md border border-line px-2 py-0.5 text-xs text-fg-muted hover:bg-elevated"
                  >
                    {p.label}
                  </button>
                </Tooltip>
              ))}
            </div>
            <p className="text-2xs text-fg-subtle">
              The best quality that fits is found first; if even a low quality is too big, the
              picture is made smaller.
            </p>
          </div>
        )}
        {flat && (
          <Row label="Behind see through pixels">
            <input
              type="color"
              aria-label="Background colour"
              value={`#${options.background
                .map((v) =>
                  Math.round(v * 255)
                    .toString(16)
                    .padStart(2, "0"),
                )
                .join("")}`}
              onChange={(e) => {
                const hex = e.target.value.slice(1);
                patch({
                  background: [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255) as [
                    number,
                    number,
                    number,
                  ],
                });
              }}
              className="h-7 w-12 rounded-md border border-line bg-raised"
            />
          </Row>
        )}
        {state.profile && options.format !== "avif" && (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={options.embedProfile}
              onChange={(e) => {
                patch({ embedProfile: e.target.checked });
              }}
              className="accent-(--accent)"
            />
            {`Keep the colour profile (${state.profile})`}
          </label>
        )}
      </div>
    </Dialog>
  );
}

export function GridDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const aids = useAids();
  if (!open) return null;
  const check = (key: keyof ViewAids, label: string) => (
    <label className="flex items-center gap-2 text-sm">
      <input
        type="checkbox"
        checked={Boolean(aids[key])}
        onChange={(e) => {
          aids.set({ [key]: e.target.checked });
        }}
        className="accent-(--accent)"
      />
      {label}
    </label>
  );
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Grid, guides and snapping"
      width="24rem"
      footer={
        <Button tooltip="Close" variant="primary" onClick={onClose}>
          Done
        </Button>
      }
    >
      <div className="flex flex-col gap-3 px-5 py-4">
        <Row label="Grid line every">
          <Num
            label="Grid spacing"
            value={aids.gridSize}
            onChange={(v) => {
              aids.set({ gridSize: Math.max(1, v) });
            }}
          />
          <span className="w-6 text-xs text-fg-subtle">px</span>
        </Row>
        <Row label="Subdivisions">
          <Num
            label="Subdivisions"
            value={aids.gridDivisions}
            max={64}
            onChange={(v) => {
              aids.set({ gridDivisions: Math.max(1, Math.round(v)) });
            }}
          />
        </Row>
        <h4 className="pt-1 text-xs font-medium text-fg-muted">Snap to</h4>
        {check("snapGuides", "Guides")}
        {check("snapGrid", "The grid, while it shows")}
        {check("snapCanvas", "Canvas edges and middle")}
        {check("snapLayers", "Other layers (smart guides)")}
        <p className="text-2xs text-fg-subtle">
          Hold Ctrl while dragging to place something without snapping.
        </p>
      </div>
    </Dialog>
  );
}
