import { clsx } from "clsx";
import { AppWindow, Copy, FolderOpen, PanelTop, Pin, Plus, X } from "lucide-react";
import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { OpenDocument } from "../bindings/OpenDocument";
import { useShortcut } from "../commands/registry";
import { IconButton } from "../components/Button";
import { RightClickMenu } from "../components/Menu";
import { MediaIcon } from "../components/MediaIcon";
import { perform, useApp } from "../stores/app";
import { pickAndOpen, shell, useTabs, useView } from "../stores/session";
import { quickToolById } from "./quickTools";

/** how far the pointer moves before a press becomes a drag */
const DRAG_START = 5;
/** how far above or below the tab bar a drag tears the tab off */
const TEAR_OFF = 44;

interface Drag {
  id: number;
  pointer: number;
  startX: number;
  startY: number;
  /** tab rectangles when the drag started */
  rects: DOMRect[];
  barTop: number;
  barBottom: number;
  moving: boolean;
}

interface DragView {
  id: number;
  offset: number;
  index: number;
  torn: boolean;
  x: number;
  y: number;
}

function insertIndex(rects: DOMRect[], x: number): number {
  const index = rects.findIndex((rect) => x < rect.left + rect.width / 2);
  return index < 0 ? rects.length : index;
}

/** the document tabs of this window */
export function Tabs() {
  const tabs = useTabs();
  const view = useView();
  const windowLabel = useApp((state) => state.windowLabel);
  const ready = useApp((state) => state.status === "ready");
  const openKey = useShortcut("file.open");
  const closeKey = useShortcut("shell.close");
  const drag = useRef<Drag | null>(null);
  const [dragView, setDragView] = useState<DragView | null>(null);
  const strip = useRef<HTMLDivElement>(null);

  const activeId = view.type === "document" ? view.id : null;

  const onPointerDown = (event: ReactPointerEvent, doc: OpenDocument) => {
    if (event.button !== 0) return;
    // tabs switch on press, like a browser, so a drag starts from the shown tab
    if (doc.id !== activeId) void shell.activate(doc.id);
    const bar = strip.current?.getBoundingClientRect();
    const rects = [...(strip.current?.querySelectorAll("[data-tab]") ?? [])].map((tab) =>
      tab.getBoundingClientRect(),
    );
    drag.current = {
      id: doc.id,
      pointer: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      rects,
      barTop: bar?.top ?? 0,
      barBottom: bar?.bottom ?? 0,
      moving: false,
    };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: ReactPointerEvent) => {
    const current = drag.current;
    if (current?.pointer !== event.pointerId) return;
    const dx = event.clientX - current.startX;
    const dy = event.clientY - current.startY;
    if (!current.moving && Math.hypot(dx, dy) < DRAG_START) return;
    current.moving = true;
    const outside =
      event.clientX < 0 ||
      event.clientY < 0 ||
      event.clientX > window.innerWidth ||
      event.clientY > window.innerHeight;
    const torn =
      outside ||
      event.clientY < current.barTop - TEAR_OFF ||
      event.clientY > current.barBottom + TEAR_OFF;
    setDragView({
      id: current.id,
      offset: dx,
      index: insertIndex(current.rects, event.clientX),
      torn,
      x: event.clientX,
      y: event.clientY,
    });
  };

  const onPointerUp = (event: ReactPointerEvent) => {
    const current = drag.current;
    if (current?.pointer !== event.pointerId) return;
    drag.current = null;
    const ended = dragView;
    setDragView(null);
    if (!current.moving || !ended) return;

    if (ended.torn) {
      void shell.toWindow(current.id, null, { x: event.screenX, y: event.screenY });
      return;
    }
    const from = tabs.findIndex((tab) => tab.id === current.id);
    // the index counts the dragged tab itself, which leaves its old place
    const to = ended.index > from ? ended.index - 1 : ended.index;
    if (to !== from) void shell.moveTab(current.id, to);
  };

  return (
    <div className="flex h-full min-w-0 flex-1 items-stretch">
      <div
        ref={strip}
        role="tablist"
        aria-label="Open files"
        className="flex min-w-0 items-stretch overflow-x-auto [scrollbar-width:none]"
      >
        {tabs.map((doc, index) => {
          const active = doc.id === activeId;
          const dragged = dragView?.id === doc.id;
          const tool = quickToolById(doc.tool);
          const showMarker = dragView && !dragView.torn && !dragged && dragView.index === index;
          return (
            <RightClickMenu.Root key={doc.id}>
              <RightClickMenu.Trigger asChild>
                <div
                  data-tab
                  role="tab"
                  tabIndex={active ? 0 : -1}
                  aria-selected={active}
                  title={doc.path ?? doc.title}
                  onPointerDown={(event) => {
                    onPointerDown(event, doc);
                  }}
                  onPointerMove={onPointerMove}
                  onPointerUp={onPointerUp}
                  onPointerCancel={() => {
                    drag.current = null;
                    setDragView(null);
                  }}
                  onAuxClick={(event) => {
                    // middle click closes, as in every browser
                    if (event.button === 1) void shell.close(doc.id);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") void shell.activate(doc.id);
                  }}
                  style={
                    dragged && !dragView.torn
                      ? { transform: `translateX(${dragView.offset}px)`, zIndex: 1 }
                      : undefined
                  }
                  className={clsx(
                    "group relative flex h-full max-w-56 min-w-28 shrink-0 items-center gap-2 border-r border-line pr-1.5 pl-3 text-sm",
                    "v-focus transition-colors duration-(--v-fast)",
                    active
                      ? "bg-surface text-ink"
                      : "bg-canvas text-ink-soft hover:bg-surface/60 hover:text-ink",
                    dragged && "shadow-pop",
                    dragged && dragView.torn && "opacity-40",
                  )}
                >
                  {active && <span className="absolute inset-x-0 top-0 h-0.5 bg-accent" />}
                  {showMarker && (
                    <span className="absolute inset-y-1.5 -left-px w-0.5 rounded bg-accent" />
                  )}
                  <MediaIcon kind={doc.kind} className="size-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">
                    {tool && <span className="text-ink-faint">{tool.title}: </span>}
                    {doc.title}
                  </span>
                  <IconButton
                    label={`Close ${doc.title}`}
                    shortcut={active ? closeKey : null}
                    className={clsx(
                      "size-5",
                      active ? "opacity-70" : "opacity-0 group-hover:opacity-70",
                      "hover:opacity-100 focus-visible:opacity-100",
                    )}
                    onPointerDown={(event) => {
                      event.stopPropagation();
                    }}
                    onClick={() => void shell.close(doc.id)}
                  >
                    <X className="size-3.5" />
                  </IconButton>
                </div>
              </RightClickMenu.Trigger>
              <TabMenu doc={doc} tabs={tabs} windowLabel={windowLabel} />
            </RightClickMenu.Root>
          );
        })}
        {dragView && !dragView.torn && dragView.index === tabs.length && (
          <span className="my-1.5 w-0.5 rounded bg-accent" />
        )}
      </div>
      <div className="flex items-center px-1.5">
        <IconButton
          label="Open file"
          shortcut={openKey}
          disabled={!ready}
          onClick={() => void pickAndOpen()}
        >
          <Plus className="size-4" />
        </IconButton>
      </div>
      {dragView?.torn && (
        <div
          className="pointer-events-none fixed z-[70] flex items-center gap-2 rounded-lg border border-accent bg-raised px-3 py-2 text-sm shadow-modal"
          style={{ left: dragView.x + 12, top: dragView.y + 12 }}
        >
          <AppWindow className="size-4 text-accent" aria-hidden />
          Release to open in a new window
        </div>
      )}
    </div>
  );
}

function TabMenu({
  doc,
  tabs,
  windowLabel,
}: {
  doc: OpenDocument;
  tabs: OpenDocument[];
  windowLabel: string;
}) {
  const path = doc.path;
  return (
    <RightClickMenu.Content>
      <RightClickMenu.Item
        onSelect={() => void shell.close(doc.id)}
        icon={<X className="size-3.5" />}
      >
        Close
      </RightClickMenu.Item>
      <RightClickMenu.Item
        disabled={tabs.length < 2}
        onSelect={() => {
          for (const other of tabs) if (other.id !== doc.id) void shell.close(other.id);
        }}
      >
        Close others
      </RightClickMenu.Item>
      <RightClickMenu.Separator />
      <RightClickMenu.Item
        icon={<AppWindow className="size-3.5" />}
        disabled={tabs.length < 2 && windowLabel !== "main"}
        onSelect={() => void shell.toWindow(doc.id, null)}
      >
        Move to new window
      </RightClickMenu.Item>
      {windowLabel !== "main" && (
        <RightClickMenu.Item
          icon={<PanelTop className="size-3.5" />}
          onSelect={() => void shell.toWindow(doc.id, "main")}
        >
          Move to main window
        </RightClickMenu.Item>
      )}
      {path && (
        <>
          <RightClickMenu.Separator />
          <RightClickMenu.Item
            icon={<FolderOpen className="size-3.5" />}
            onSelect={() => void perform((api) => api.files.reveal(path))}
          >
            Show in folder
          </RightClickMenu.Item>
          <RightClickMenu.Item
            icon={<Copy className="size-3.5" />}
            onSelect={() => void copyText(path)}
          >
            Copy path
          </RightClickMenu.Item>
          <RightClickMenu.Item
            icon={<Pin className="size-3.5" />}
            onSelect={() => void perform((api) => api.recent.pin(path, true))}
          >
            Pin to home
          </RightClickMenu.Item>
        </>
      )}
    </RightClickMenu.Content>
  );
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    useApp.getState().setNotice("The path could not be copied to the clipboard.");
  }
}
