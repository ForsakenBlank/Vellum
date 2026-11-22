import { Save, Trash2 } from "lucide-react";
import { useState } from "react";

import type { ImageState } from "../../bindings/ImageState";
import { IconButton } from "../../components/Button";
import { Tooltip } from "../../components/Tooltip";
import { ipc } from "../../lib/ipc";
import { combineFor, fromHex, hsvToRgb, rgbToHsv, toHex } from "./geometry";
import { type Rgb, useImageTools } from "./store";

/** a colour square, a hue strip and a hex field for the foreground or background */
function Picker({ value, onChange }: { value: Rgb; onChange: (c: Rgb) => void }) {
  const [h, s, v] = rgbToHsv(value);
  const [hue, setHue] = useState(h);
  const shownHue = s > 0 && v > 0 ? h : hue;
  const [draft, setDraft] = useState<string | null>(null);
  const pickSv = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const ns = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    const nv = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height));
    onChange(hsvToRgb(shownHue, ns, nv));
  };
  const base = toHex(hsvToRgb(shownHue, 1, 1));
  return (
    <div className="flex flex-col gap-1.5">
      <div
        role="slider"
        aria-label="Saturation and brightness"
        aria-valuetext={toHex(value)}
        aria-valuenow={Math.round(v * 100)}
        tabIndex={0}
        className="relative h-24 cursor-crosshair rounded-md"
        style={{
          background: `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, ${base})`,
        }}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          pickSv(e);
        }}
        onPointerMove={(e) => {
          if (e.buttons & 1) pickSv(e);
        }}
      >
        <span
          className="pointer-events-none absolute size-2.5 -translate-x-1/2 translate-y-1/2 rounded-full border-2 border-white shadow-1"
          style={{ left: `${String(s * 100)}%`, bottom: `${String(v * 100)}%` }}
        />
      </div>
      <input
        type="range"
        min={0}
        max={359}
        value={Math.round(shownHue * 360)}
        aria-label="Hue"
        onChange={(e) => {
          const nh = Number(e.target.value) / 360;
          setHue(nh);
          onChange(hsvToRgb(nh, s || 1, v || 1));
        }}
        className="hue-slider h-2 w-full appearance-none rounded-full"
        style={{
          background: "linear-gradient(to right, #f00, #ff0, #0f0, #0ff, #00f, #f0f, #f00)",
        }}
      />
      <input
        value={draft ?? toHex(value)}
        aria-label="Colour as hex"
        onChange={(e) => {
          setDraft(e.target.value);
          const c = fromHex(e.target.value);
          if (c) onChange(c);
        }}
        onBlur={() => {
          setDraft(null);
        }}
        className="h-6 rounded-md border border-line bg-raised px-1.5 font-mono text-xs uppercase text-fg"
      />
    </div>
  );
}

export function ColorPanel() {
  const t = useImageTools();
  const [editing, setEditing] = useState<"foreground" | "background">("foreground");
  return (
    <section aria-label="Colour" className="flex flex-col gap-2 border-b border-line p-2.5">
      <div className="flex items-center gap-2">
        {(["foreground", "background"] as const).map((which) => (
          <Tooltip
            key={which}
            text={
              which === "foreground"
                ? "Foreground colour, what brushes paint"
                : "Background colour, what gradients end on"
            }
          >
            <button
              type="button"
              aria-label={
                which === "foreground" ? "Edit the foreground colour" : "Edit the background colour"
              }
              aria-pressed={editing === which}
              onClick={() => {
                setEditing(which);
              }}
              className={`size-7 rounded-md border-2 ${editing === which ? "border-accent" : "border-line-strong"}`}
              style={{ background: toHex(t[which]) }}
            />
          </Tooltip>
        ))}
        <span className="text-xs text-fg-muted">
          {editing === "foreground" ? "Foreground" : "Background"}
        </span>
      </div>
      <Picker
        value={t[editing]}
        onChange={(c) => {
          t.set({ [editing]: c });
        }}
      />
    </section>
  );
}

/** saved selections, photoshop's alpha channels */
export function ChannelsPanel({
  doc,
  state,
  fail,
}: {
  doc: string;
  state: ImageState;
  fail: (err: unknown) => void;
}) {
  const command = (kind: string, params: unknown) => {
    ipc.imageCommand(doc, kind, params).catch(fail);
  };
  return (
    <section aria-label="Saved selections" className="flex flex-col border-t border-line">
      <div className="flex items-center gap-1 px-2.5 pt-2 pb-1">
        <h3 className="flex-1 text-xs font-medium text-fg-muted">Saved selections</h3>
        <IconButton
          size="sm"
          label="Save the selection"
          command="image.select.save"
          disabled={!state.selection}
          icon={<Save className="size-3.5" />}
          onClick={() => {
            command("image.select.save", {});
          }}
        />
      </div>
      {state.channels.length === 0 ? (
        <p className="px-2.5 pb-2.5 text-2xs text-fg-subtle">
          Select something and save it to come back to it later.
        </p>
      ) : (
        <ul className="max-h-32 overflow-y-auto pb-1">
          {state.channels.map((c) => (
            <li key={c.id} className="flex items-center gap-1 px-2.5 hover:bg-elevated">
              <Tooltip text="Load it. Shift adds to the selection, Alt takes away, both keep the overlap">
                <button
                  type="button"
                  className="h-7 min-w-0 flex-1 truncate text-left text-xs"
                  onClick={(e) => {
                    command("image.select.load", {
                      channel: c.id,
                      mode: combineFor("replace", e.shiftKey, e.altKey),
                    });
                  }}
                >
                  {c.name}
                </button>
              </Tooltip>
              <IconButton
                size="sm"
                label={`Delete ${c.name}`}
                icon={<Trash2 className="size-3.5" />}
                onClick={() => {
                  command("image.channel.remove", { channel: c.id });
                }}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
