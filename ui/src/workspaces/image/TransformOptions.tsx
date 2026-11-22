import { Check, FlipHorizontal2, FlipVertical2, Ruler, X } from "lucide-react";

import type { ImageState } from "../../bindings/ImageState";
import type { Interpolation } from "../../bindings/Interpolation";
import { IconButton } from "../../components/Button";
import { Tooltip } from "../../components/Tooltip";
import { useSessions } from "./session";
import { useImageTools } from "./store";
import {
  ASPECTS,
  type HandleMode,
  type Pt,
  flipQuad,
  fromAffine,
  isParallelogram,
  perspectiveSize,
  toAffine,
  warpFromQuad,
} from "./transformMath";
import { cancelTransform, commitCrop, commitTransform, startCrop } from "./transformTool";

// the options bar while free transform or the crop tool is in use

const MODES: [HandleMode | "warp", string, string][] = [
  ["scale", "Scale", "Corners scale, Shift lets the shape go, Alt scales about the middle"],
  ["rotate", "Rotate", "Drag anywhere to turn about the pivot, Shift in 15 degree steps"],
  ["skew", "Skew", "Edges slide along themselves. Ctrl and an edge does this too"],
  ["distort", "Distort", "Corners move on their own. Ctrl and a corner does this too"],
  [
    "perspective",
    "Perspective",
    "A corner and its neighbour move apart together. Ctrl Alt Shift and a corner does this too",
  ],
  ["warp", "Warp", "Bend the picture with sixteen points"],
];

const INTERPOLATION: [Interpolation, string][] = [
  ["bicubic", "Bicubic"],
  ["bilinear", "Bilinear"],
  ["nearest", "Nearest"],
];

function Field({
  label,
  tip,
  value,
  unit,
  disabled,
  onChange,
}: {
  label: string;
  tip: string;
  value: number;
  unit: string;
  disabled: boolean;
  onChange: (v: number) => void;
}) {
  return (
    <Tooltip text={tip}>
      <label className="flex items-center gap-1 text-2xs text-fg-subtle">
        {label}
        <input
          type="number"
          step="any"
          disabled={disabled}
          aria-label={tip}
          value={Number(value.toFixed(2))}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (Number.isFinite(v)) onChange(v);
          }}
          className="h-6 w-14 rounded-md border border-line bg-raised px-1 text-right text-xs text-fg tabular-nums disabled:opacity-50"
        />
        <span className="-ml-0.5 text-xs">{unit}</span>
      </label>
    </Tooltip>
  );
}

export function TransformOptions({ fail }: { fail: (err: unknown) => void }) {
  const t = useSessions((s) => s.transform);
  const patch = useSessions((s) => s.patchTransform);
  if (!t) return null;
  const affine = !t.warp && isParallelogram(t.corners);
  const a = toAffine(t.source, t.corners);
  const setAffine = (p: Partial<typeof a>) => {
    const next = { ...a, ...p };
    const corners = fromAffine(t.source, next);
    patch({ corners, pivot: [next.x, next.y] });
  };
  const flip = (horizontal: boolean) => {
    if (t.warp) {
      const w = t.warp;
      const flipped = w.map((_, i) => {
        const col = i % 4;
        const row = Math.floor(i / 4);
        return w[horizontal ? row * 4 + (3 - col) : (3 - row) * 4 + col] as Pt;
      });
      patch({ warp: flipped });
    } else {
      patch({ corners: flipQuad(t.corners, horizontal) });
    }
  };
  return (
    <>
      <div
        role="radiogroup"
        aria-label="What dragging does"
        className="flex rounded-md border border-line bg-raised p-0.5"
      >
        {MODES.map(([id, text, tip]) => {
          const on = id === "warp" ? t.warp !== null : t.warp === null && t.mode === id;
          return (
            <Tooltip key={id} text={tip}>
              <button
                type="button"
                role="radio"
                aria-checked={on}
                onClick={() => {
                  if (id === "warp") patch({ warp: t.warp ?? warpFromQuad(t.source, t.corners) });
                  else patch({ mode: id, warp: null });
                }}
                className={`h-5 rounded-sm px-2 text-2xs ${on ? "bg-accent-soft text-accent-text" : "text-fg-muted hover:text-fg"}`}
              >
                {text}
              </button>
            </Tooltip>
          );
        })}
      </div>
      <div className="flex items-center gap-2">
        <Field
          label="X"
          tip="Where the middle is, across"
          value={a.x}
          unit="px"
          disabled={!affine}
          onChange={(x) => {
            setAffine({ x });
          }}
        />
        <Field
          label="Y"
          tip="Where the middle is, down"
          value={a.y}
          unit="px"
          disabled={!affine}
          onChange={(y) => {
            setAffine({ y });
          }}
        />
        <Field
          label="W"
          tip="Width, percent of what it was"
          value={a.w}
          unit="%"
          disabled={!affine}
          onChange={(w) => {
            setAffine({ w });
          }}
        />
        <Field
          label="H"
          tip="Height, percent of what it was"
          value={a.h}
          unit="%"
          disabled={!affine}
          onChange={(h) => {
            setAffine({ h });
          }}
        />
        <Field
          label="Angle"
          tip="Turn, clockwise"
          value={a.angle}
          unit="°"
          disabled={!affine}
          onChange={(angle) => {
            setAffine({ angle });
          }}
        />
        <Field
          label="Skew"
          tip="How far the sides lean"
          value={a.skew}
          unit="°"
          disabled={!affine}
          onChange={(skew) => {
            setAffine({ skew });
          }}
        />
      </div>
      <IconButton
        size="sm"
        label="Flip horizontal"
        icon={<FlipHorizontal2 className="size-3.5" />}
        onClick={() => {
          flip(true);
        }}
      />
      <IconButton
        size="sm"
        label="Flip vertical"
        icon={<FlipVertical2 className="size-3.5" />}
        onClick={() => {
          flip(false);
        }}
      />
      <Tooltip text="How pixels are picked: bicubic is smooth and sharp, nearest keeps hard pixels">
        <select
          aria-label="Interpolation"
          value={t.interpolation}
          onChange={(e) => {
            patch({ interpolation: e.target.value as Interpolation });
          }}
          className="h-6 rounded-md border border-line bg-raised px-1 text-xs text-fg"
        >
          {INTERPOLATION.map(([id, text]) => (
            <option key={id} value={id}>
              {text}
            </option>
          ))}
        </select>
      </Tooltip>
      <span className="ml-auto flex items-center gap-1">
        <IconButton
          size="sm"
          label="Cancel the transform (Esc)"
          icon={<X className="size-3.5" />}
          onClick={cancelTransform}
        />
        <IconButton
          size="sm"
          label="Apply the transform (Enter)"
          icon={<Check className="size-3.5" />}
          onClick={() => {
            void commitTransform(fail);
          }}
        />
      </span>
    </>
  );
}

export function CropOptions({ state, fail }: { state: ImageState; fail: (err: unknown) => void }) {
  const c = useSessions((s) => s.crop);
  const patch = useSessions((s) => s.patchCrop);
  const setCrop = useSessions((s) => s.setCrop);
  const kind = useImageTools((s) => s.cropKind);
  const setTools = useImageTools((s) => s.set);
  if (!c) return null;
  const ratio = c.aspect < 0 ? state.width / state.height : c.aspect;
  const applyAspect = (aspect: number) => {
    const r = aspect < 0 ? state.width / state.height : aspect;
    if (r <= 0 || c.quad) {
      patch({ aspect });
      return;
    }
    // the same area, reshaped about its middle, then kept inside the picture
    const area = c.box.w * c.box.h;
    let w = Math.sqrt(area * r);
    let h = w / r;
    const f = Math.min(1, state.width / w, state.height / h);
    w *= f;
    h *= f;
    patch({ aspect, box: { ...c.box, w, h } });
  };
  const [w, h] = c.quad ? perspectiveSize(c.quad) : [Math.round(c.box.w), Math.round(c.box.h)];
  return (
    <>
      <div
        role="radiogroup"
        aria-label="Crop kind"
        className="flex rounded-md border border-line bg-raised p-0.5"
      >
        {(
          [
            ["crop", "Crop", "A box, which can be turned to straighten the picture"],
            ["perspective", "Perspective", "Four free corners, straightened into a rectangle"],
          ] as const
        ).map(([id, text, tip]) => (
          <Tooltip key={id} text={tip} shortcut="Shift+C">
            <button
              type="button"
              role="radio"
              aria-checked={kind === id}
              onClick={() => {
                setTools({ cropKind: id });
                setCrop(startCrop(c.doc, state, id === "perspective"));
              }}
              className={`h-5 rounded-sm px-2 text-2xs ${kind === id ? "bg-accent-soft text-accent-text" : "text-fg-muted hover:text-fg"}`}
            >
              {text}
            </button>
          </Tooltip>
        ))}
      </div>
      {!c.quad && (
        <>
          <label className="flex items-center gap-1.5 text-2xs text-fg-subtle">
            Ratio
            <select
              value={String(c.aspect)}
              onChange={(e) => {
                applyAspect(Number(e.target.value));
              }}
              className="h-6 rounded-md border border-line bg-raised px-1 text-xs text-fg"
            >
              {ASPECTS.map(([name, r]) => (
                <option key={name} value={String(r)}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          {ratio > 0 && (
            <Tooltip text="Swap width and height">
              <button
                type="button"
                onClick={() => {
                  patch({ aspect: 1 / ratio, box: { ...c.box, w: c.box.h, h: c.box.w } });
                }}
                className="h-6 rounded-md border border-line px-2 text-2xs text-fg-muted hover:bg-elevated"
              >
                Swap
              </button>
            </Tooltip>
          )}
          <IconButton
            size="sm"
            label="Straighten: draw a line along something that should be level"
            active={c.straighten}
            icon={<Ruler className="size-3.5" />}
            onClick={() => {
              patch({ straighten: !c.straighten });
            }}
          />
          <Field
            label="Angle"
            tip="How far the crop is turned"
            value={c.box.angle}
            unit="°"
            disabled={false}
            onChange={(angle) => {
              patch({ box: { ...c.box, angle } });
            }}
          />
        </>
      )}
      <Tooltip text="Throw away what is outside the crop. Off keeps it beyond the canvas, so it can come back with canvas size">
        <label className="flex items-center gap-1 text-xs text-fg-muted">
          <input
            type="checkbox"
            checked={c.deleteOutside}
            onChange={(e) => {
              patch({ deleteOutside: e.target.checked });
            }}
            className="accent-(--accent)"
          />
          Delete cropped pixels
        </label>
      </Tooltip>
      <span className="text-2xs text-fg-subtle tabular-nums">{`${String(w)} × ${String(h)} px`}</span>
      <span className="ml-auto flex items-center gap-1">
        <IconButton
          size="sm"
          label="Start over (Esc)"
          icon={<X className="size-3.5" />}
          onClick={() => {
            setCrop(startCrop(c.doc, state, c.quad !== null));
          }}
        />
        <IconButton
          size="sm"
          label="Crop (Enter)"
          icon={<Check className="size-3.5" />}
          onClick={() => {
            void commitCrop(fail);
          }}
        />
      </span>
    </>
  );
}
