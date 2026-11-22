import type { ReactNode } from "react";

import type { Combine } from "../../bindings/Combine";
import type { GradientKind } from "../../bindings/GradientKind";
import { Tooltip } from "../../components/Tooltip";
import { type ToolState, useImageTools } from "./store";

// the bar along the top of the canvas with the current tool's options, like photoshop's

function Num({
  label,
  tip,
  value,
  min,
  max,
  step = 1,
  unit = "",
  onChange,
}: {
  label: string;
  tip: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  return (
    <Tooltip text={tip}>
      <label className="flex items-center gap-1.5 text-2xs text-fg-subtle">
        {label}
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          aria-label={label}
          onChange={(e) => {
            onChange(Number(e.target.value));
          }}
          className="w-20 accent-(--accent)"
        />
        <input
          type="number"
          min={min}
          max={max}
          step={step}
          value={Number(value.toFixed(2))}
          aria-label={`${label} value`}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (Number.isFinite(v)) onChange(Math.min(max, Math.max(min, v)));
          }}
          className="h-6 w-14 rounded-md border border-line bg-raised px-1 text-right text-xs text-fg tabular-nums"
        />
        {unit && <span className="-ml-1 text-xs text-fg-subtle">{unit}</span>}
      </label>
    </Tooltip>
  );
}

function Check({
  label,
  tip,
  checked,
  onChange,
}: {
  label: string;
  tip: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <Tooltip text={tip}>
      <label className="flex items-center gap-1 text-xs text-fg-muted">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => {
            onChange(e.target.checked);
          }}
          className="accent-(--accent)"
        />
        {label}
      </label>
    </Tooltip>
  );
}

function Pick<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: [T, string][];
  onChange: (v: T) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex rounded-md border border-line bg-raised p-0.5"
    >
      {options.map(([id, text]) => (
        <button
          key={id}
          type="button"
          role="radio"
          aria-checked={value === id}
          onClick={() => {
            onChange(id);
          }}
          className={`h-5 rounded-sm px-2 text-2xs ${value === id ? "bg-accent-soft text-accent-text" : "text-fg-muted hover:text-fg"}`}
        >
          {text}
        </button>
      ))}
    </div>
  );
}

const MODES: [Combine, string][] = [
  ["replace", "New"],
  ["add", "Add"],
  ["subtract", "Subtract"],
  ["intersect", "Intersect"],
];

function selectionOptions(t: ToolState, set: (patch: Partial<ToolState>) => void): ReactNode {
  return (
    <>
      <Pick
        label="Selection mode"
        value={t.selectMode}
        options={MODES}
        onChange={(v) => {
          set({ selectMode: v });
        }}
      />
      <Check
        label="Anti alias"
        tip="Soften the edge by a pixel"
        checked={t.antiAlias}
        onChange={(v) => {
          set({ antiAlias: v });
        }}
      />
    </>
  );
}

export function OptionsBar() {
  const t = useImageTools();
  const set = t.set;
  const b = t.tool === "eraser" ? t.eraser : t.brush;
  let body: ReactNode = null;
  switch (t.tool) {
    case "brush":
    case "eraser":
      body = (
        <>
          {t.tool === "brush" && (
            <Pick
              label="Brush or pencil"
              value={t.brushKind}
              options={[
                ["brush", "Brush"],
                ["pencil", "Pencil"],
              ]}
              onChange={(v) => {
                set({ brushKind: v });
              }}
            />
          )}
          <Num
            label="Size"
            tip="Brush size. [ and ] change it"
            value={b.size}
            min={1}
            max={2000}
            unit="px"
            onChange={(v) => {
              t.setBrush({ size: v });
            }}
          />
          <Num
            label="Hardness"
            tip="Soft or hard edge. Shift [ and Shift ] change it"
            value={Math.round(b.hardness * 100)}
            min={0}
            max={100}
            unit="%"
            onChange={(v) => {
              t.setBrush({ hardness: v / 100 });
            }}
          />
          <Num
            label="Opacity"
            tip="The most one stroke can cover"
            value={Math.round(b.opacity * 100)}
            min={1}
            max={100}
            unit="%"
            onChange={(v) => {
              t.setBrush({ opacity: v / 100 });
            }}
          />
          <Num
            label="Flow"
            tip="How much each dab adds, building up as you go over a spot"
            value={Math.round(b.flow * 100)}
            min={1}
            max={100}
            unit="%"
            onChange={(v) => {
              t.setBrush({ flow: v / 100 });
            }}
          />
          <Num
            label="Smoothing"
            tip="Steadies the line, with a little lag"
            value={Math.round(b.smoothing * 100)}
            min={0}
            max={100}
            unit="%"
            onChange={(v) => {
              t.setBrush({ smoothing: v / 100 });
            }}
          />
          <Num
            label="Spacing"
            tip="Distance between dabs, as a share of the size"
            value={Math.round(b.spacing * 100)}
            min={1}
            max={200}
            unit="%"
            onChange={(v) => {
              t.setBrush({ spacing: v / 100 });
            }}
          />
          <Num
            label="Scatter"
            tip="How far dabs stray from the line"
            value={Math.round(b.scatter * 100)}
            min={0}
            max={500}
            unit="%"
            onChange={(v) => {
              t.setBrush({ scatter: v / 100, count: v > 0 ? Math.max(b.count, 2) : 1 });
            }}
          />
          <Num
            label="Texture"
            tip="Paper grain showing through the paint"
            value={Math.round(b.textureDepth * 100)}
            min={0}
            max={100}
            unit="%"
            onChange={(v) => {
              t.setBrush({ textureDepth: v / 100 });
            }}
          />
          <Check
            label="Pressure size"
            tip="Pen pressure changes the size"
            checked={b.pressureSize}
            onChange={(v) => {
              t.setBrush({ pressureSize: v });
            }}
          />
          <Check
            label="Pressure opacity"
            tip="Pen pressure changes the opacity"
            checked={b.pressureOpacity}
            onChange={(v) => {
              t.setBrush({ pressureOpacity: v });
            }}
          />
          <Check
            label="Pressure flow"
            tip="Pen pressure changes the flow"
            checked={b.pressureFlow}
            onChange={(v) => {
              t.setBrush({ pressureFlow: v });
            }}
          />
          <Check
            label="Tilt"
            tip="Tilting the pen flattens and turns the tip"
            checked={b.tilt}
            onChange={(v) => {
              t.setBrush({ tilt: v });
            }}
          />
        </>
      );
      break;
    case "marquee":
      body = (
        <>
          <Pick
            label="Shape"
            value={t.marquee}
            options={[
              ["rect", "Rectangle"],
              ["ellipse", "Ellipse"],
            ]}
            onChange={(v) => {
              set({ marquee: v });
            }}
          />
          {selectionOptions(t, set)}
          <Num
            label="Feather"
            tip="Soften the new selection's edge"
            value={t.feather}
            min={0}
            max={250}
            unit="px"
            onChange={(v) => {
              set({ feather: v });
            }}
          />
        </>
      );
      break;
    case "lasso":
      body = (
        <>
          <Pick
            label="Lasso"
            value={t.lasso}
            options={[
              ["free", "Lasso"],
              ["polygon", "Polygon"],
              ["magnetic", "Magnetic"],
            ]}
            onChange={(v) => {
              set({ lasso: v });
            }}
          />
          {selectionOptions(t, set)}
          <Num
            label="Feather"
            tip="Soften the new selection's edge"
            value={t.feather}
            min={0}
            max={250}
            unit="px"
            onChange={(v) => {
              set({ feather: v });
            }}
          />
          {t.lasso === "magnetic" && (
            <>
              <Num
                label="Width"
                tip="How far from the pointer the lasso looks for an edge"
                value={t.magneticWidth}
                min={1}
                max={64}
                unit="px"
                onChange={(v) => {
                  set({ magneticWidth: v });
                }}
              />
              <Num
                label="Contrast"
                tip="How strong an edge must be to follow it"
                value={Math.round(t.magneticContrast * 100)}
                min={5}
                max={100}
                unit="%"
                onChange={(v) => {
                  set({ magneticContrast: v / 100 });
                }}
              />
            </>
          )}
          {t.lasso !== "free" && (
            <span className="text-2xs text-fg-subtle">
              Click to add corners, double click or Enter to finish, Esc to stop
            </span>
          )}
        </>
      );
      break;
    case "wand":
      body = (
        <>
          <Pick
            label="Wand"
            value={t.wand}
            options={[
              ["wand", "Magic wand"],
              ["quick", "Quick selection"],
            ]}
            onChange={(v) => {
              set({ wand: v });
            }}
          />
          {t.wand === "wand" ? (
            <>
              {selectionOptions(t, set)}
              <Num
                label="Tolerance"
                tip="How different a colour may be and still be picked, 0 to 255"
                value={t.tolerance}
                min={0}
                max={255}
                onChange={(v) => {
                  set({ tolerance: v });
                }}
              />
              <Check
                label="Contiguous"
                tip="Only pick pixels touching the one clicked"
                checked={t.contiguous}
                onChange={(v) => {
                  set({ contiguous: v });
                }}
              />
            </>
          ) : (
            <Num
              label="Size"
              tip="Brush size of the quick selection. Alt takes away"
              value={t.quickSize}
              min={2}
              max={500}
              unit="px"
              onChange={(v) => {
                set({ quickSize: v });
              }}
            />
          )}
          <Check
            label="Sample all layers"
            tip="Look at every layer, not just the current one"
            checked={t.sampleAll}
            onChange={(v) => {
              set({ sampleAll: v });
            }}
          />
        </>
      );
      break;
    case "gradient":
      body = (
        <>
          <Pick
            label="Fill tool"
            value={t.fill}
            options={[
              ["gradient", "Gradient"],
              ["bucket", "Paint bucket"],
            ]}
            onChange={(v) => {
              set({ fill: v });
            }}
          />
          {t.fill === "gradient" ? (
            <>
              <Pick<GradientKind>
                label="Gradient type"
                value={t.gradientKind}
                options={[
                  ["linear", "Linear"],
                  ["radial", "Radial"],
                  ["angle", "Angle"],
                  ["reflected", "Reflected"],
                  ["diamond", "Diamond"],
                ]}
                onChange={(v) => {
                  set({ gradientKind: v });
                }}
              />
              <Check
                label="To transparent"
                tip="Fade the foreground colour out instead of into the background colour"
                checked={t.gradientToClear}
                onChange={(v) => {
                  set({ gradientToClear: v });
                }}
              />
              <Check
                label="Reverse"
                tip="Swap the ends of the gradient"
                checked={t.gradientReverse}
                onChange={(v) => {
                  set({ gradientReverse: v });
                }}
              />
              <Check
                label="Dither"
                tip="A touch of noise so smooth gradients do not band"
                checked={t.gradientDither}
                onChange={(v) => {
                  set({ gradientDither: v });
                }}
              />
              <Num
                label="Opacity"
                tip="How strongly the gradient covers"
                value={Math.round(t.gradientOpacity * 100)}
                min={1}
                max={100}
                unit="%"
                onChange={(v) => {
                  set({ gradientOpacity: v / 100 });
                }}
              />
            </>
          ) : (
            <>
              <Num
                label="Tolerance"
                tip="How different a colour may be and still be filled, 0 to 255"
                value={t.bucket.tolerance}
                min={0}
                max={255}
                onChange={(v) => {
                  set({ bucket: { ...t.bucket, tolerance: v } });
                }}
              />
              <Check
                label="Contiguous"
                tip="Only fill pixels touching the one clicked"
                checked={t.bucket.contiguous}
                onChange={(v) => {
                  set({ bucket: { ...t.bucket, contiguous: v } });
                }}
              />
              <Check
                label="Anti alias"
                tip="Soften the fill's edge by a pixel"
                checked={t.bucket.antiAlias}
                onChange={(v) => {
                  set({ bucket: { ...t.bucket, antiAlias: v } });
                }}
              />
              <Check
                label="All layers"
                tip="Pick the area from every layer, not just the current one"
                checked={t.bucket.sampleAll}
                onChange={(v) => {
                  set({ bucket: { ...t.bucket, sampleAll: v } });
                }}
              />
              <Num
                label="Opacity"
                tip="How strongly the fill covers"
                value={Math.round(t.bucket.opacity * 100)}
                min={1}
                max={100}
                unit="%"
                onChange={(v) => {
                  set({ bucket: { ...t.bucket, opacity: v / 100 } });
                }}
              />
            </>
          )}
        </>
      );
      break;
    case "eyedropper":
      body = (
        <>
          <label className="flex items-center gap-1.5 text-2xs text-fg-subtle">
            Sample size
            <select
              value={t.sampleSize}
              onChange={(e) => {
                set({ sampleSize: Number(e.target.value) });
              }}
              className="h-6 rounded-md border border-line bg-raised px-1 text-xs text-fg"
            >
              {[1, 3, 5, 11, 31, 51, 101].map((n) => (
                <option key={n} value={n}>
                  {n === 1 ? "Point sample" : `${String(n)} by ${String(n)} average`}
                </option>
              ))}
            </select>
          </label>
          <Pick
            label="Sample"
            value={t.sampleLayers}
            options={[
              ["current", "Current layer"],
              ["all", "All layers"],
            ]}
            onChange={(v) => {
              set({ sampleLayers: v });
            }}
          />
        </>
      );
      break;
    case "move":
      body = (
        <span className="text-2xs text-fg-subtle">
          Drag to move the current layer. Shift keeps it straight, arrow keys nudge a pixel (Shift
          for ten).
        </span>
      );
      break;
    case "hand":
      body = (
        <span className="text-2xs text-fg-subtle">
          Drag to move around the picture. Hold R to turn the view.
        </span>
      );
      break;
    case "zoom":
      body = (
        <span className="text-2xs text-fg-subtle">Click to zoom in, Alt click to zoom out.</span>
      );
      break;
  }
  return (
    <div
      role="toolbar"
      aria-label="Tool options"
      className="flex min-h-9 shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line bg-panel px-3 py-1"
    >
      {body}
    </div>
  );
}
