import {
  ChevronDown,
  ChevronRight,
  CornerLeftDown,
  Copy,
  Eye,
  EyeOff,
  FolderPlus,
  Lock,
  Merge,
  Move,
  Paintbrush,
  Plus,
  Search,
  SquareDashed,
  Trash2,
  Type,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { ImageState } from "../../bindings/ImageState";
import type { Locks } from "../../bindings/Locks";
import { IconButton } from "../../components/Button";
import { Tooltip } from "../../components/Tooltip";
import { ipc } from "../../lib/ipc";
import { type LayerRow, type Thumb, findLayer, layerRows, thumbKey } from "./geometry";
import { BLEND_MODES, LABELS } from "./labels";

interface Props {
  doc: string;
  state: ImageState;
  thumbs: Map<string, Thumb>;
  fail: (err: unknown) => void;
  /** a layer whose mask is shown on its own, alt clicking its mask thumbnail */
  maskView: number | null;
  setMaskView: (id: number | null) => void;
}

function ThumbCanvas({
  thumb,
  label,
  active,
}: {
  thumb: Thumb | undefined;
  label: string;
  active: boolean;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c || !thumb) return;
    c.width = thumb.width;
    c.height = thumb.height;
    c.getContext("2d")?.putImageData(new ImageData(thumb.pixels, thumb.width, thumb.height), 0, 0);
  }, [thumb]);
  return (
    <span
      className={`checker flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-sm border ${active ? "border-accent" : "border-line"}`}
    >
      <canvas ref={ref} aria-label={label} className="max-h-full max-w-full object-contain" />
    </span>
  );
}

export function LayersPanel({ doc, state, thumbs, fail, maskView, setMaskView }: Props) {
  const [query, setQuery] = useState("");
  const [renaming, setRenaming] = useState<number | null>(null);
  const [drag, setDrag] = useState<{
    id: number;
    over: number | null;
    where: "above" | "below" | "into";
  } | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const rows = layerRows(state.layers, query);
  const active = state.active === null ? undefined : findLayer(state.layers, state.active);

  const command = (kind: string, params: unknown = {}) => {
    ipc.imageCommand(doc, kind, params).catch(fail);
  };
  const props = (id: number, patch: Record<string, unknown>) => {
    command("image.layer.props", { id, ...patch });
  };
  const seal = () => {
    ipc.sealDocument(doc).catch(() => undefined);
  };
  const select = (id: number, target: "pixels" | "mask") => {
    command("image.layer.select", { id, target });
  };

  const drop = () => {
    if (!drag || drag.over === null || drag.over === drag.id) {
      setDrag(null);
      return;
    }
    const target = rows.find((r) => r.layer.id === drag.over);
    setDrag(null);
    if (!target) return;
    if (drag.where === "into" && target.layer.group) {
      command("image.layer.move", {
        id: drag.id,
        parent: target.layer.id,
        index: target.layer.children.length,
      });
      return;
    }
    // rows run top first, so "above" a row means one higher in its list
    const index = drag.where === "above" ? target.index + 1 : target.index;
    command("image.layer.move", { id: drag.id, parent: target.parent, index });
  };

  const rowAt = (clientY: number): { row: LayerRow; where: "above" | "below" | "into" } | null => {
    const items = listRef.current?.querySelectorAll<HTMLElement>("[data-layer]");
    for (const el of items ?? []) {
      const r = el.getBoundingClientRect();
      if (clientY < r.top || clientY >= r.bottom) continue;
      const row = rows.find((x) => String(x.layer.id) === el.dataset.layer);
      if (!row) return null;
      const t = (clientY - r.top) / r.height;
      const where = row.layer.group && t > 0.3 && t < 0.7 ? "into" : t < 0.5 ? "above" : "below";
      return { row, where };
    }
    return null;
  };

  const locks = active?.locks;
  const setLock = (key: keyof Locks) => {
    if (!active || !locks) return;
    props(active.id, { locks: { ...locks, [key]: !locks[key] } });
  };

  return (
    <section aria-label="Layers" className="flex min-h-64 flex-1 flex-col">
      <div className="flex flex-col gap-1.5 border-b border-line px-2.5 py-2">
        <div className="flex items-center gap-1.5">
          <select
            aria-label="Blend mode"
            disabled={!active}
            value={
              active?.group && active.passThrough ? "passThrough" : (active?.blend ?? "normal")
            }
            onChange={(e) => {
              if (!active) return;
              if (e.target.value === "passThrough") props(active.id, { passThrough: true });
              else props(active.id, { blend: e.target.value });
            }}
            className="h-(--row-sm) min-w-0 flex-1 rounded-md border border-line bg-raised px-1.5 text-xs text-fg"
          >
            {active?.group && <option value="passThrough">Pass Through</option>}
            {BLEND_MODES.map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
          <Percent
            label="Opacity"
            value={active?.opacity}
            onChange={(v) => {
              if (active) props(active.id, { opacity: v });
            }}
            onCommit={seal}
          />
        </div>
        <div className="flex items-center gap-1">
          <span className="text-2xs text-fg-subtle">Lock</span>
          <IconButton
            size="sm"
            label="Lock see through pixels"
            active={!!locks?.transparency}
            disabled={!active}
            icon={<SquareDashed className="size-3.5" />}
            onClick={() => {
              setLock("transparency");
            }}
          />
          <IconButton
            size="sm"
            label="Lock painting"
            active={!!locks?.pixels}
            disabled={!active}
            icon={<Paintbrush className="size-3.5" />}
            onClick={() => {
              setLock("pixels");
            }}
          />
          <IconButton
            size="sm"
            label="Lock position"
            active={!!locks?.position}
            disabled={!active}
            icon={<Move className="size-3.5" />}
            onClick={() => {
              setLock("position");
            }}
          />
          <IconButton
            size="sm"
            label="Lock everything"
            active={!!locks?.all}
            disabled={!active}
            icon={<Lock className="size-3.5" />}
            onClick={() => {
              setLock("all");
            }}
          />
          <span className="flex-1" />
          <Percent
            label="Fill"
            value={active?.fill}
            onChange={(v) => {
              if (active) props(active.id, { fill: v });
            }}
            onCommit={seal}
          />
        </div>
        <label className="flex h-(--row-sm) items-center gap-1.5 rounded-md border border-line bg-raised px-1.5 focus-within:border-accent-line">
          <Search className="size-3.5 text-fg-subtle" />
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
            }}
            placeholder="Find layers by name, colour or kind"
            aria-label="Find layers"
            className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-fg-subtle"
          />
        </label>
      </div>

      <div
        ref={listRef}
        role="tree"
        aria-label="Layer list"
        className="min-h-0 flex-1 overflow-y-auto py-1"
        onPointerMove={(e) => {
          if (!drag) return;
          const hit = rowAt(e.clientY);
          setDrag({ ...drag, over: hit?.row.layer.id ?? null, where: hit?.where ?? "above" });
        }}
        onPointerUp={drop}
        onPointerLeave={() => {
          if (drag) setDrag(null);
        }}
      >
        {rows.length === 0 && (
          <p className="px-3 py-6 text-center text-xs text-fg-subtle">
            {query ? "No layer matches." : "No layers yet."}
          </p>
        )}
        {rows.map((row) => {
          const l = row.layer;
          const selected = l.id === state.active;
          const over = drag?.over === l.id && drag.id !== l.id;
          const label = LABELS.find(([id]) => id === l.label);
          return (
            <div
              key={l.id}
              role="treeitem"
              aria-selected={selected}
              aria-expanded={l.group ? l.expanded : undefined}
              data-layer={l.id}
              tabIndex={selected ? 0 : -1}
              onPointerDown={(e) => {
                if (e.button !== 0 || (e.target as HTMLElement).closest("button,input")) return;
                const startY = e.clientY;
                const onMove = (m: PointerEvent) => {
                  if (Math.abs(m.clientY - startY) > 4) {
                    setDrag({ id: l.id, over: null, where: "above" });
                    window.removeEventListener("pointermove", onMove);
                  }
                };
                window.addEventListener("pointermove", onMove);
                window.addEventListener(
                  "pointerup",
                  () => {
                    window.removeEventListener("pointermove", onMove);
                  },
                  { once: true },
                );
              }}
              onClick={(e) => {
                if ((e.target as HTMLElement).closest("button,input")) return;
                select(l.id, "pixels");
              }}
              onKeyDown={(e) => {
                if (e.key === "F2") setRenaming(l.id);
              }}
              style={{ paddingLeft: `${String(6 + row.depth * 14)}px` }}
              className={`relative flex h-11 items-center gap-1.5 pr-2 text-xs ${selected ? "bg-accent-soft" : "hover:bg-elevated"} ${l.visible ? "" : "opacity-60"} ${over && drag.where === "into" ? "ring-1 ring-accent ring-inset" : ""}`}
            >
              {over && drag.where !== "into" && (
                <span
                  className={`absolute inset-x-1 h-0.5 rounded-full bg-accent ${drag.where === "above" ? "top-0" : "bottom-0"}`}
                />
              )}
              {label && l.label !== "none" && (
                <span
                  aria-hidden
                  className="absolute inset-y-1 left-0.5 w-1 rounded-full"
                  style={{ background: label[2] }}
                />
              )}
              <IconButton
                size="sm"
                label={l.visible ? "Hide this layer" : "Show this layer"}
                icon={l.visible ? <Eye className="size-3.5" /> : <EyeOff className="size-3.5" />}
                onClick={() => {
                  props(l.id, { visible: !l.visible });
                }}
              />
              {l.group ? (
                <IconButton
                  size="sm"
                  label={l.expanded ? "Close the group" : "Open the group"}
                  icon={
                    l.expanded ? (
                      <ChevronDown className="size-3.5" />
                    ) : (
                      <ChevronRight className="size-3.5" />
                    )
                  }
                  onClick={() => {
                    props(l.id, { expanded: !l.expanded });
                  }}
                />
              ) : l.clipped ? (
                <Tooltip text="Clipped to the layer below">
                  <CornerLeftDown
                    aria-label="Clipped to the layer below"
                    className="size-3.5 shrink-0 text-fg-subtle"
                  />
                </Tooltip>
              ) : null}
              <button
                type="button"
                aria-label={`${l.name} pixels`}
                onClick={() => {
                  select(l.id, "pixels");
                }}
              >
                <ThumbCanvas
                  thumb={thumbs.get(thumbKey(l.id, false))}
                  label={`${l.name} preview`}
                  active={selected && state.target === "pixels" && l.mask !== null}
                />
              </button>
              {l.mask && (
                <Tooltip text="Click to paint the mask. Alt click to see it on its own, Shift click to turn it off.">
                  <button
                    type="button"
                    aria-label={`${l.name} mask`}
                    onClick={(e) => {
                      if (e.altKey) setMaskView(maskView === l.id ? null : l.id);
                      else if (e.shiftKey) props(l.id, { maskEnabled: !l.mask?.enabled });
                      else select(l.id, "mask");
                    }}
                    className={l.mask.enabled ? "" : "opacity-40"}
                  >
                    <ThumbCanvas
                      thumb={thumbs.get(thumbKey(l.id, true))}
                      label={`${l.name} mask preview`}
                      active={selected && state.target === "mask"}
                    />
                  </button>
                </Tooltip>
              )}
              {renaming === l.id ? (
                <input
                  autoFocus
                  defaultValue={l.name}
                  aria-label="Layer name"
                  onBlur={(e) => {
                    setRenaming(null);
                    const name = e.target.value.trim();
                    if (name && name !== l.name) props(l.id, { name });
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") setRenaming(null);
                  }}
                  className="min-w-0 flex-1 rounded-sm bg-raised px-1 outline-none focus:shadow-(--ring)"
                />
              ) : (
                <span
                  className="min-w-0 flex-1 truncate"
                  title="Double click to rename"
                  onDoubleClick={() => {
                    setRenaming(l.id);
                  }}
                >
                  {l.name}
                </span>
              )}
              {l.origin === "text" && (
                <Tooltip text="Imported text, kept as pixels">
                  <Type aria-label="Imported text" className="size-3.5 shrink-0 text-fg-subtle" />
                </Tooltip>
              )}
              {(l.locks.all || l.locks.pixels || l.locks.position || l.locks.transparency) && (
                <Lock aria-label="Locked" className="size-3 shrink-0 text-fg-subtle" />
              )}
              {l.blend !== "normal" && (
                <span className="shrink-0 text-2xs text-fg-subtle">
                  {BLEND_MODES.find(([m]) => m === l.blend)?.[1]}
                </span>
              )}
            </div>
          );
        })}
      </div>

      <div className="flex items-center gap-0.5 border-t border-line px-1.5 py-1">
        <select
          aria-label="Colour label"
          disabled={!active}
          value={active?.label ?? "none"}
          onChange={(e) => {
            if (active) props(active.id, { label: e.target.value });
          }}
          className="h-6 w-20 rounded-md border border-line bg-raised px-1 text-2xs text-fg"
        >
          {LABELS.map(([id, name]) => (
            <option key={id} value={id}>
              {name}
            </option>
          ))}
        </select>
        <span className="flex-1" />
        <IconButton
          size="sm"
          label="Clipping mask"
          command="image.layer.clip"
          active={!!active?.clipped}
          disabled={!active}
          icon={<CornerLeftDown className="size-3.5" />}
          onClick={() => {
            if (active) props(active.id, { clipped: !active.clipped });
          }}
        />
        <IconButton
          size="sm"
          label="Add layer mask"
          command="image.layer.add_mask"
          disabled={!active || active.mask !== null}
          icon={<SquareDashed className="size-3.5" />}
          onClick={() => {
            command("image.mask.add", { reveal: true });
          }}
        />
        <IconButton
          size="sm"
          label="New group"
          icon={<FolderPlus className="size-3.5" />}
          onClick={() => {
            command("image.layer.add", { group: true });
          }}
        />
        <IconButton
          size="sm"
          label="New layer"
          command="image.layer.new"
          icon={<Plus className="size-3.5" />}
          onClick={() => {
            command("image.layer.add");
          }}
        />
        <IconButton
          size="sm"
          label="Duplicate layer"
          disabled={!active}
          icon={<Copy className="size-3.5" />}
          onClick={() => {
            command("image.layer.duplicate");
          }}
        />
        <IconButton
          size="sm"
          label="Merge down"
          command="image.layer.merge_down"
          disabled={!active}
          icon={<Merge className="size-3.5" />}
          onClick={() => {
            command("image.layer.merge_down");
          }}
        />
        <IconButton
          size="sm"
          label={state.target === "mask" ? "Delete the layer mask" : "Delete layer"}
          disabled={!active}
          icon={<Trash2 className="size-3.5" />}
          onClick={() => {
            if (state.target === "mask") command("image.mask.remove");
            else command("image.layer.remove");
          }}
        />
      </div>
    </section>
  );
}

/** a percentage with a slider underneath, for opacity and fill */
function Percent({
  label,
  value,
  onChange,
  onCommit,
}: {
  label: string;
  value: number | undefined;
  onChange: (v: number) => void;
  onCommit: () => void;
}) {
  const shown = Math.round((value ?? 1) * 100);
  return (
    <label className="flex items-center gap-1 text-2xs text-fg-subtle">
      {label}
      <input
        type="range"
        min={0}
        max={100}
        value={shown}
        disabled={value === undefined}
        aria-label={label}
        onChange={(e) => {
          onChange(Number(e.target.value) / 100);
        }}
        onPointerUp={onCommit}
        onKeyUp={onCommit}
        className="w-16 accent-(--accent)"
      />
      <span className="w-8 text-right text-xs text-fg tabular-nums">{`${String(shown)}%`}</span>
    </label>
  );
}
