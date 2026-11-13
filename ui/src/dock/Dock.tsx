import { clsx } from "clsx";
import {
  ArrowDownToLine,
  ArrowLeftToLine,
  ArrowRightToLine,
  MoreHorizontal,
  X,
} from "lucide-react";
import { Fragment, useRef, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";

import type { DockLayout } from "../bindings/DockLayout";
import type { PanelStack } from "../bindings/PanelStack";
import { IconButton } from "../components/Button";
import { Menu } from "../components/Menu";
import type { PanelDef, Workspace } from "../workspaces/types";
import {
  activatePanel,
  movePanel,
  removePanel,
  resizeZone,
  shareStacks,
  ZONES,
  type DropSpot,
  type ZoneName,
} from "./layout";
import { changeLayout, saveLayout, useDock, useLayout } from "./state";

const DRAG_START = 5;

function remPx(): number {
  return parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
}

/** works out where a dragged panel would land from what is under the pointer */
function spotAt(x: number, y: number): DropSpot | null {
  const element = document.elementFromPoint(x, y);
  const edge = element?.closest<HTMLElement>("[data-dock-edge]");
  if (edge) {
    const zone = edge.dataset.dockEdge as ZoneName;
    return { zone, stack: Number(edge.dataset.stacks ?? 0), place: "before" };
  }
  const stackElement = element?.closest<HTMLElement>("[data-dock-stack]");
  if (!stackElement) return null;
  const [zone, index] = (stackElement.dataset.dockStack ?? "").split(":");
  if (!zone || index === undefined) return null;
  const stack = Number(index);
  if (element?.closest("[data-dock-header]")) {
    return { zone: zone as ZoneName, stack, place: "into" };
  }
  const rect = stackElement.getBoundingClientRect();
  const before =
    zone === "bottom" ? x < rect.left + rect.width / 2 : y < rect.top + rect.height / 2;
  return { zone: zone as ZoneName, stack, place: before ? "before" : "after" };
}

function sameSpot(a: DropSpot | null, b: DropSpot | null): boolean {
  return a?.zone === b?.zone && a?.stack === b?.stack && a?.place === b?.place;
}

interface DockProps {
  workspace: Workspace;
  /** the workspace's own content, in the middle */
  children: ReactNode;
}

/**
 * the frame around a workspace: panels docked left, right and below the
 * middle, each in tabbed stacks that can be resized, rearranged and closed
 */
export function Dock({ workspace, children }: DockProps) {
  const layout = useLayout(workspace);
  const dragging = useDock((state) => state.drag !== null);
  const panels = new Map(workspace.panels.map((panel) => [panel.id, panel]));

  return (
    <div className="relative flex h-full min-h-0 min-w-0">
      <Zone zone="left" layout={layout} workspace={workspace} panels={panels} />
      <div className="relative flex min-w-0 flex-1 flex-col">
        <div className="relative min-h-0 flex-1">{children}</div>
        <Zone zone="bottom" layout={layout} workspace={workspace} panels={panels} />
        {dragging && <Edges layout={layout} />}
      </div>
      <Zone zone="right" layout={layout} workspace={workspace} panels={panels} />
      <DragGhost />
    </div>
  );
}

/** drop strips along the edges of the middle, to dock into a zone from scratch */
function Edges({ layout }: { layout: DockLayout }) {
  const spot = useDock((state) => state.drag?.spot ?? null);
  const place: Record<ZoneName, string> = {
    left: "inset-y-0 left-0 w-6",
    right: "inset-y-0 right-0 w-6",
    bottom: "inset-x-0 bottom-0 h-6",
  };
  return (
    <>
      {ZONES.map((zone) => {
        const stacks = layout[zone].open ? layout[zone].stacks.length : 0;
        const lit = spot?.zone === zone && spot.stack === stacks && spot.place === "before";
        return (
          <div
            key={zone}
            data-dock-edge={zone}
            data-stacks={stacks}
            className={clsx(
              "absolute z-20 transition-colors duration-(--v-fast)",
              place[zone],
              lit ? "bg-accent/40" : "bg-accent/10",
            )}
          />
        );
      })}
    </>
  );
}

function DragGhost() {
  const drag = useDock((state) => state.drag);
  if (!drag) return null;
  return (
    <div
      className="pointer-events-none fixed z-[70] rounded-md border border-accent bg-raised px-2.5 py-1 text-xs shadow-modal"
      style={{ left: drag.x + 10, top: drag.y + 10 }}
    >
      {drag.title}
    </div>
  );
}

interface ZoneProps {
  zone: ZoneName;
  layout: DockLayout;
  workspace: Workspace;
  panels: Map<string, PanelDef>;
}

function Zone({ zone, layout, workspace, panels }: ZoneProps) {
  const state = layout[zone];
  const ref = useRef<HTMLDivElement>(null);
  if (!state.open || state.stacks.length === 0) return null;
  const vertical = zone !== "bottom";

  const startResize = (event: ReactPointerEvent) => {
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const start = { x: event.clientX, y: event.clientY, size: state.size };
    const rem = remPx();
    let latest = layout;
    const onMove = (move: PointerEvent) => {
      const delta =
        zone === "left"
          ? move.clientX - start.x
          : zone === "right"
            ? start.x - move.clientX
            : start.y - move.clientY;
      latest = resizeZone(layout, zone, start.size + delta / rem);
      useDock.getState().setPreview({ workspace: workspace.id, layout: latest });
    };
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      void saveLayout(workspace, latest);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  };

  const startShare = (event: ReactPointerEvent, first: number) => {
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const items = ref.current?.querySelectorAll<HTMLElement>("[data-dock-stack]");
    const a = items?.[first]?.getBoundingClientRect();
    const b = items?.[first + 1]?.getBoundingClientRect();
    if (!a || !b) return;
    let latest = layout;
    const onMove = (move: PointerEvent) => {
      const fraction = vertical
        ? (move.clientY - a.top) / (b.bottom - a.top)
        : (move.clientX - a.left) / (b.right - a.left);
      latest = shareStacks(layout, zone, first, fraction);
      useDock.getState().setPreview({ workspace: workspace.id, layout: latest });
    };
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      void saveLayout(workspace, latest);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  };

  const label = { left: "Left panels", right: "Right panels", bottom: "Bottom panels" }[zone];

  return (
    <aside
      ref={ref}
      aria-label={label}
      className={clsx(
        "relative flex shrink-0 bg-surface",
        vertical ? "flex-col" : "flex-row",
        zone === "left" && "border-r border-line",
        zone === "right" && "border-l border-line",
        zone === "bottom" && "border-t border-line",
      )}
      style={vertical ? { width: `${state.size}rem` } : { height: `${state.size}rem` }}
    >
      {state.stacks.map((stack, index) => (
        <Fragment key={stack.panels.join(",")}>
          {index > 0 && (
            <div
              role="separator"
              aria-orientation={vertical ? "horizontal" : "vertical"}
              onPointerDown={(event) => {
                startShare(event, index - 1);
              }}
              className={clsx(
                "shrink-0 bg-line transition-colors duration-(--v-fast) hover:bg-accent",
                vertical ? "h-px cursor-row-resize" : "w-px cursor-col-resize",
                "relative before:absolute before:content-['']",
                vertical
                  ? "before:inset-x-0 before:-inset-y-1"
                  : "before:inset-y-0 before:-inset-x-1",
              )}
            />
          )}
          <Stack
            zone={zone}
            index={index}
            stack={stack}
            layout={layout}
            workspace={workspace}
            panels={panels}
          />
        </Fragment>
      ))}
      <div
        role="separator"
        aria-label={`Resize ${label.toLowerCase()}`}
        aria-orientation={vertical ? "vertical" : "horizontal"}
        onPointerDown={startResize}
        className={clsx(
          "absolute z-10 transition-colors duration-(--v-fast) hover:bg-accent/60",
          zone === "left" && "inset-y-0 -right-1 w-2 cursor-col-resize",
          zone === "right" && "inset-y-0 -left-1 w-2 cursor-col-resize",
          zone === "bottom" && "inset-x-0 -top-1 h-2 cursor-row-resize",
        )}
      />
    </aside>
  );
}

interface StackProps {
  zone: ZoneName;
  index: number;
  stack: PanelStack;
  layout: DockLayout;
  workspace: Workspace;
  panels: Map<string, PanelDef>;
}

function Stack({ zone, index, stack, layout, workspace, panels }: StackProps) {
  const spot = useDock((state) => state.drag?.spot ?? null);
  const active = panels.get(stack.active ?? "") ?? panels.get(stack.panels[0] ?? "");
  const Body = active?.component;
  const here = spot?.zone === zone && spot.stack === index ? spot : null;

  const startDrag = (event: ReactPointerEvent, panel: PanelDef) => {
    if (event.button !== 0) return;
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const start = { x: event.clientX, y: event.clientY };
    let moving = false;
    const onMove = (move: PointerEvent) => {
      if (!moving && Math.hypot(move.clientX - start.x, move.clientY - start.y) < DRAG_START) {
        return;
      }
      moving = true;
      const next = spotAt(move.clientX, move.clientY);
      const drag = useDock.getState().drag;
      if (drag && sameSpot(drag.spot, next) && Math.abs(drag.x - move.clientX) < 2) return;
      useDock.getState().setDrag({
        panel: panel.id,
        title: panel.title,
        x: move.clientX,
        y: move.clientY,
        spot: next,
      });
    };
    const finish = (commit: boolean) => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      handle.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKey);
      const drag = useDock.getState().drag;
      useDock.getState().setDrag(null);
      if (!moving) {
        void changeLayout(workspace, (current) => activatePanel(current, panel.id));
      } else if (commit && drag?.spot) {
        const target = drag.spot;
        void changeLayout(workspace, (current) => movePanel(current, panel.id, target));
      }
    };
    const onUp = () => {
      finish(true);
    };
    const onCancel = () => {
      finish(false);
    };
    const onKey = (key: KeyboardEvent) => {
      if (key.key === "Escape") finish(false);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
    handle.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKey);
  };

  const moveTo = (target: ZoneName) => {
    if (!active) return;
    const id = active.id;
    void changeLayout(workspace, (current) =>
      movePanel(current, id, {
        zone: target,
        stack: current[target].stacks.length,
        place: "before",
      }),
    );
  };

  return (
    <section
      data-dock-stack={`${zone}:${index}`}
      aria-label={active?.title}
      className="relative flex min-h-0 min-w-0 flex-col"
      style={{ flex: `${stack.weight} 1 0` }}
    >
      <header
        data-dock-header
        className={clsx(
          "flex h-8 shrink-0 items-stretch border-b border-line bg-canvas/40 pr-1",
          here?.place === "into" && "bg-accent-soft",
        )}
      >
        <div
          role="tablist"
          aria-label="Panels"
          className="flex min-w-0 flex-1 items-stretch overflow-x-auto [scrollbar-width:none]"
        >
          {stack.panels.map((id) => {
            const panel = panels.get(id);
            if (!panel) return null;
            const selected = panel.id === active?.id;
            return (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={selected}
                onPointerDown={(event) => {
                  startDrag(event, panel);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    void changeLayout(workspace, (current) => activatePanel(current, id));
                  }
                }}
                className={clsx(
                  "v-focus relative flex shrink-0 items-center gap-1.5 px-2.5 text-xs font-medium whitespace-nowrap",
                  selected ? "text-ink" : "text-ink-faint hover:text-ink-soft",
                )}
              >
                <panel.icon className="size-3.5" aria-hidden />
                {panel.title}
                {selected && (
                  <span className="absolute inset-x-2 bottom-0 h-0.5 rounded bg-accent" />
                )}
              </button>
            );
          })}
        </div>
        <Menu.Root>
          <Menu.Trigger asChild>
            <IconButton label="Panel options" className="my-auto size-6">
              <MoreHorizontal className="size-3.5" />
            </IconButton>
          </Menu.Trigger>
          <Menu.Content align="end">
            {zone !== "left" && (
              <Menu.Item
                icon={<ArrowLeftToLine className="size-3.5" />}
                onSelect={() => {
                  moveTo("left");
                }}
              >
                Dock on the left
              </Menu.Item>
            )}
            {zone !== "right" && (
              <Menu.Item
                icon={<ArrowRightToLine className="size-3.5" />}
                onSelect={() => {
                  moveTo("right");
                }}
              >
                Dock on the right
              </Menu.Item>
            )}
            {zone !== "bottom" && (
              <Menu.Item
                icon={<ArrowDownToLine className="size-3.5" />}
                onSelect={() => {
                  moveTo("bottom");
                }}
              >
                Dock at the bottom
              </Menu.Item>
            )}
            <Menu.Separator />
            <Menu.Item
              icon={<X className="size-3.5" />}
              disabled={!active}
              onSelect={() => {
                if (active) void saveLayout(workspace, removePanel(layout, active.id));
              }}
            >
              Close {active?.title}
            </Menu.Item>
          </Menu.Content>
        </Menu.Root>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{Body && <Body />}</div>
      {here && here.place !== "into" && (
        <div
          className={clsx(
            "pointer-events-none absolute z-20 bg-accent/25 ring-2 ring-accent ring-inset",
            zone === "bottom"
              ? here.place === "before"
                ? "inset-y-0 left-0 w-1/2"
                : "inset-y-0 right-0 w-1/2"
              : here.place === "before"
                ? "inset-x-0 top-0 h-1/2"
                : "inset-x-0 bottom-0 h-1/2",
          )}
        />
      )}
    </section>
  );
}
