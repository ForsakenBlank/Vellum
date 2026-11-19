import { X } from "lucide-react";
import { useRef, useState, type ReactNode, type PointerEvent as ReactPointerEvent } from "react";

import type { DockArea } from "../bindings/DockArea";
import type { DockGroup } from "../bindings/DockGroup";
import type { DockLayout } from "../bindings/DockLayout";
import { IconButton } from "../components/Button";
import { Tooltip } from "../components/Tooltip";
import {
  movePanel,
  resizeArea,
  resizeGroups,
  setActive,
  type AreaName,
  type DockDrop,
} from "../lib/dock";
import { useSettings } from "../state/settings";
import { currentLayout, hidePanel, setLayout } from "./actions";
import { HistoryPanel } from "./panels/HistoryPanel";
import { InfoPanel } from "./panels/InfoPanel";
import { JobsPanel } from "./panels/JobsPanel";
import { panelInfo } from "./panels/registry";

const panelViews: Record<string, () => ReactNode> = {
  info: () => <InfoPanel />,
  history: () => <HistoryPanel />,
  jobs: () => <JobsPanel />,
};

interface DragState {
  panel: string;
  x: number;
  y: number;
  drop: DockDrop | null;
}

/**
 * lays the three docks out around the document area. panels are dragged by their tab with
 * pointer events (not html drag and drop, which tauri's file drop handling turns off on
 * windows). drop on a tab strip to join that group, or on a dock edge for a new group.
 */
export function DockFrame({ children }: { children: ReactNode }) {
  const settings = useSettings((s) => s.settings);
  const layout = currentLayout(settings);
  const [drag, setDrag] = useState<DragState | null>(null);
  // a live layout while resizing, saved once on pointer up
  const [preview, setPreview] = useState<DockLayout | null>(null);
  const shown = preview ?? layout;

  const startPanelDrag = (panel: string, event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let started = false;
    const move = (e: PointerEvent) => {
      if (!started && Math.hypot(e.clientX - startX, e.clientY - startY) < 5) return;
      started = true;
      setDrag({ panel, x: e.clientX, y: e.clientY, drop: dropAt(e.clientX, e.clientY) });
    };
    const up = (e: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (!started) return;
      const drop = dropAt(e.clientX, e.clientY);
      if (drop) setLayout(movePanel(currentLayout(), panel, drop));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const area = (name: AreaName) => (
    <DockAreaView
      name={name}
      area={shown[name]}
      dragging={drag !== null}
      dropTarget={drag?.drop ?? null}
      onPanelDrag={startPanelDrag}
      onResize={(size, done) => {
        const next = resizeArea(layout, name, size);
        if (done) {
          setPreview(null);
          setLayout(next);
        } else setPreview(next);
      }}
      onResizeGroups={(index, weights, done) => {
        const next = resizeGroups(layout, name, index, weights);
        if (done) {
          setPreview(null);
          setLayout(next);
        } else setPreview(next);
      }}
    />
  );

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1">
      {area("left")}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="relative min-h-0 flex-1">{children}</div>
        {area("bottom")}
      </div>
      {area("right")}
      {drag && <DragGhost drag={drag} />}
    </div>
  );
}

/** reads the drop target under the pointer from data attributes on the dock elements */
function dropAt(x: number, y: number): DockDrop | null {
  for (const el of document.elementsFromPoint(x, y)) {
    if (!(el instanceof HTMLElement)) continue;
    const { dockTabs, dockEdge } = el.dataset;
    if (dockTabs) {
      const [area, group] = dockTabs.split(":") as [AreaName, string];
      const tabs = [...el.querySelectorAll<HTMLElement>("[data-dock-tab]")];
      let index = tabs.length;
      for (const [i, tab] of tabs.entries()) {
        const rect = tab.getBoundingClientRect();
        if (x < rect.left + rect.width / 2) {
          index = i;
          break;
        }
      }
      return { type: "tab", area, group, index };
    }
    if (dockEdge) {
      const [area, index] = dockEdge.split(":") as [AreaName, string];
      return { type: "group", area, index: Number(index) };
    }
  }
  return null;
}

function DragGhost({ drag }: { drag: DragState }) {
  const info = panelInfo(drag.panel);
  return (
    <div
      className="pointer-events-none fixed z-[var(--z-drop)] flex items-center gap-1.5 rounded-md bg-elevated px-2 py-1 text-xs shadow-pop"
      style={{ left: drag.x + 12, top: drag.y + 8 }}
    >
      {info && <info.icon className="size-3.5" />}
      {info?.title}
      <span className="text-fg-subtle">
        {drag.drop
          ? drag.drop.type === "tab"
            ? "· add as a tab"
            : "· new group"
          : "· drop on a dock"}
      </span>
    </div>
  );
}

interface DockAreaViewProps {
  name: AreaName;
  area: DockArea;
  dragging: boolean;
  dropTarget: DockDrop | null;
  onPanelDrag: (panel: string, event: ReactPointerEvent) => void;
  onResize: (size: number, done: boolean) => void;
  onResizeGroups: (index: number, weights: [number, number], done: boolean) => void;
}

function DockAreaView({
  name,
  area,
  dragging,
  dropTarget,
  onPanelDrag,
  onResize,
  onResizeGroups,
}: DockAreaViewProps) {
  const vertical = name !== "bottom";
  const visible = !area.collapsed && area.groups.length > 0;
  const ref = useRef<HTMLDivElement>(null);

  // while dragging a panel, a hidden or empty dock shows a slim strip to drop onto
  if (!visible) {
    if (!dragging) return null;
    const hot = dropTarget?.type === "group" && dropTarget.area === name;
    return (
      <div
        data-dock-edge={`${name}:0`}
        className={`shrink-0 border-dashed border-accent-line transition-colors ${vertical ? "w-10 border-x" : "h-10 border-y"} ${hot ? "bg-accent-soft" : "bg-canvas/60"}`}
      />
    );
  }

  const startResize = (event: ReactPointerEvent) => {
    event.preventDefault();
    const start = vertical ? event.clientX : event.clientY;
    const scale = parseFloat(getComputedStyle(document.documentElement).fontSize) / 16 || 1;
    const sign = name === "left" ? 1 : -1;
    const from = area.size;
    const sizeAt = (e: PointerEvent) =>
      from + (sign * ((vertical ? e.clientX : e.clientY) - start)) / scale;
    const move = (e: PointerEvent) => {
      onResize(sizeAt(e), false);
    };
    const up = (e: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      onResize(sizeAt(e), true);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const startGroupResize = (index: number, event: ReactPointerEvent) => {
    event.preventDefault();
    const box = ref.current;
    const a = area.groups[index];
    const b = area.groups[index + 1];
    if (!box || !a || !b) return;
    const total = area.groups.reduce((sum, g) => sum + g.weight, 0);
    const length = vertical ? box.clientHeight : box.clientWidth;
    const pair = a.weight + b.weight;
    const start = vertical ? event.clientY : event.clientX;
    const weightsAt = (e: PointerEvent): [number, number] => {
      const delta = (((vertical ? e.clientY : e.clientX) - start) / length) * total;
      const first = Math.min(pair - 0.1, Math.max(0.1, a.weight + delta));
      return [first, pair - first];
    };
    const move = (e: PointerEvent) => {
      onResizeGroups(index, weightsAt(e), false);
    };
    const up = (e: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      onResizeGroups(index, weightsAt(e), true);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const sizeStyle = vertical
    ? { width: `${area.size / 16}rem` }
    : { height: `${area.size / 16}rem` };
  const edge = name === "left" ? "border-r" : name === "right" ? "border-l" : "border-t";
  const handle =
    name === "left"
      ? "-right-1 top-0 h-full w-2 cursor-col-resize"
      : name === "right"
        ? "-left-1 top-0 h-full w-2 cursor-col-resize"
        : "-top-1 left-0 h-2 w-full cursor-row-resize";

  return (
    <aside
      aria-label={`${name} dock`}
      style={sizeStyle}
      className={`relative flex shrink-0 bg-panel ${edge} border-line ${vertical ? "flex-col" : "flex-row"}`}
    >
      <div
        role="separator"
        aria-label={`Resize the ${name} dock`}
        aria-orientation={vertical ? "vertical" : "horizontal"}
        onPointerDown={startResize}
        className={`absolute z-[var(--z-dock)] ${handle} transition-colors hover:bg-accent-line`}
      />
      <div
        ref={ref}
        className={`flex min-h-0 min-w-0 flex-1 ${vertical ? "flex-col" : "flex-row"}`}
      >
        {area.groups.map((group, index) => (
          <GroupSlot
            key={group.id}
            name={name}
            index={index}
            group={group}
            vertical={vertical}
            dragging={dragging}
            dropTarget={dropTarget}
            last={index === area.groups.length - 1}
            onPanelDrag={onPanelDrag}
            onSplitDrag={startGroupResize}
          />
        ))}
      </div>
    </aside>
  );
}

interface GroupSlotProps {
  name: AreaName;
  index: number;
  group: DockGroup;
  vertical: boolean;
  dragging: boolean;
  dropTarget: DockDrop | null;
  last: boolean;
  onPanelDrag: (panel: string, event: ReactPointerEvent) => void;
  onSplitDrag: (index: number, event: ReactPointerEvent) => void;
}

function GroupSlot({
  name,
  index,
  group,
  vertical,
  dragging,
  dropTarget,
  last,
  onPanelDrag,
  onSplitDrag,
}: GroupSlotProps) {
  const edgeHot = (at: number) =>
    dropTarget?.type === "group" && dropTarget.area === name && dropTarget.index === at;
  const edgeClass = (at: number) =>
    `${vertical ? "h-2 w-full" : "w-2 h-full"} shrink-0 ${edgeHot(at) ? "bg-accent" : dragging ? "bg-accent-soft" : ""}`;

  return (
    <>
      {dragging && <div data-dock-edge={`${name}:${index}`} className={edgeClass(index)} />}
      <div
        className="flex min-h-0 min-w-0 flex-col"
        style={{ flexGrow: group.weight, flexBasis: 0 }}
      >
        <PanelGroup area={name} group={group} dropTarget={dropTarget} onPanelDrag={onPanelDrag} />
      </div>
      {dragging && last && (
        <div data-dock-edge={`${name}:${index + 1}`} className={edgeClass(index + 1)} />
      )}
      {!dragging && !last && (
        <div
          role="separator"
          aria-label="Resize panel groups"
          onPointerDown={(event) => {
            onSplitDrag(index, event);
          }}
          className={`shrink-0 bg-line transition-colors hover:bg-accent-line ${vertical ? "h-px cursor-row-resize py-0.5 bg-clip-content" : "w-px cursor-col-resize px-0.5 bg-clip-content"}`}
        />
      )}
    </>
  );
}

interface PanelGroupProps {
  area: AreaName;
  group: DockGroup;
  dropTarget: DockDrop | null;
  onPanelDrag: (panel: string, event: ReactPointerEvent) => void;
}

function PanelGroup({ area, group, dropTarget, onPanelDrag }: PanelGroupProps) {
  const hot = dropTarget?.type === "tab" && dropTarget.group === group.id;
  const view = panelViews[group.active];
  return (
    <>
      <div
        role="tablist"
        aria-label="Panels"
        data-dock-tabs={`${area}:${group.id}`}
        className={`group/dock flex h-(--row) shrink-0 items-center gap-0.5 border-b border-line px-1.5 ${hot ? "bg-accent-soft" : ""}`}
      >
        {group.panels.map((panel, i) => {
          const info = panelInfo(panel);
          if (!info) return null;
          const active = panel === group.active;
          const marker = hot && dropTarget.index === i;
          return (
            <div key={panel} data-dock-tab className="group/tab relative flex items-center">
              {marker && <span className="absolute -left-0.5 h-4 w-0.5 rounded-full bg-accent" />}
              <Tooltip
                text={`${info.title}. Drag to move it to another dock.`}
                command={info.command}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={active}
                  onPointerDown={(event) => {
                    onPanelDrag(panel, event);
                  }}
                  onClick={() => {
                    setLayout(setActive(currentLayout(), group.id, panel));
                  }}
                  className={`flex h-6 items-center gap-1.5 rounded-sm px-2 text-xs font-medium transition-colors duration-(--motion-fast) ${
                    active ? "bg-elevated text-fg" : "text-fg-subtle hover:text-fg"
                  }`}
                >
                  <info.icon className="size-3.5" strokeWidth={1.8} />
                  {info.title}
                </button>
              </Tooltip>
            </div>
          );
        })}
        {hot && dropTarget.index >= group.panels.length && (
          <span className="h-4 w-0.5 rounded-full bg-accent" />
        )}
        <span className="flex-1" />
        <IconButton
          size="sm"
          label={`Close the ${panelInfo(group.active)?.title ?? ""} panel`}
          icon={<X className="size-3" />}
          className="opacity-0 group-hover/dock:opacity-100 focus-visible:opacity-100 hover:opacity-100"
          onClick={() => {
            hidePanel(group.active);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">{view ? view() : null}</div>
    </>
  );
}
