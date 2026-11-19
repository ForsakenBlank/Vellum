import { ContextMenu } from "radix-ui";
import { House, X } from "lucide-react";
import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { DocumentInfo } from "../bindings/DocumentInfo";
import { menuContent, menuItem, menuSeparator } from "../components/menuStyles";
import { Tooltip } from "../components/Tooltip";
import { ipc } from "../lib/ipc";
import { isMainWindow, windowLabel } from "../lib/window";
import { tabsIn, useDocuments } from "../state/documents";
import { useUi } from "../state/ui";
import { quickTool } from "../workspaces/quickTools";
import { workspaceInfo } from "../workspaces/workspaces";
import { closeTab, moveToMain, tearOff } from "./actions";

interface Drag {
  id: string;
  /** where the tab would land, counted among this window's tabs */
  index: number;
  outside: boolean;
}

/**
 * the tab strip. tabs reorder by dragging with the pointer; dragging one out of the window
 * opens it in a window of its own. pointer events are used instead of html drag and drop,
 * which conflicts with tauri's file drop handling on windows.
 */
export function DocumentTabs() {
  const documents = useDocuments((s) => s.documents);
  const activeId = useDocuments((s) => s.activeId);
  const emptyWorkspace = useDocuments((s) => s.emptyWorkspace);
  const activate = useDocuments((s) => s.activate);
  const showHome = useDocuments((s) => s.showHome);
  const tabs = tabsIn(documents);
  const strip = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);

  const indexAt = (x: number) => {
    const els = strip.current?.querySelectorAll<HTMLElement>("[data-doc-tab]") ?? [];
    let index = els.length;
    for (const [i, el] of [...els].entries()) {
      const rect = el.getBoundingClientRect();
      if (x < rect.left + rect.width / 2) {
        index = i;
        break;
      }
    }
    return index;
  };

  const startDrag = (doc: DocumentInfo, event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let started = false;
    let last: Drag | null = null;
    const isOutside = (e: PointerEvent) =>
      e.clientX < 0 ||
      e.clientY < 0 ||
      e.clientX > window.innerWidth ||
      e.clientY > window.innerHeight;
    const move = (e: PointerEvent) => {
      if (!started && Math.hypot(e.clientX - startX, e.clientY - startY) < 6) return;
      started = true;
      last = { id: doc.id, index: indexAt(e.clientX), outside: isOutside(e) };
      setDrag(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (!started || !last) return;
      if (last.outside) {
        void tearOff(doc.id);
        return;
      }
      const from = tabs.findIndex((t) => t.id === doc.id);
      if (last.index === from || last.index === from + 1) return;
      // the index rust wants is the position after the tab has been taken out
      const target = last.index > from ? last.index - 1 : last.index;
      ipc.moveDocument(doc.id, windowLabel, target).catch((err: unknown) => {
        useUi
          .getState()
          .toast({ level: "error", title: "The tab could not move", body: String(err) });
      });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const homeActive = activeId === null && emptyWorkspace === null;

  return (
    <div
      ref={strip}
      role="tablist"
      aria-label="Open documents"
      className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto [scrollbar-width:none]"
    >
      {isMainWindow && (
        <Tooltip text="Home: recent files and quick tools" command="app.home">
          <button
            type="button"
            role="tab"
            aria-selected={homeActive}
            aria-label="Home"
            onClick={showHome}
            className={`flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors duration-(--motion-fast) ${
              homeActive
                ? "bg-raised text-fg shadow-1"
                : "text-fg-muted hover:bg-raised/60 hover:text-fg"
            }`}
          >
            <House className="size-3.5" strokeWidth={1.8} />
            Home
          </button>
        </Tooltip>
      )}
      {tabs.map((doc, i) => (
        <Tab
          key={doc.id}
          doc={doc}
          active={doc.id === activeId}
          dragging={drag?.id === doc.id}
          markerBefore={drag !== null && !drag.outside && drag.index === i && drag.id !== doc.id}
          markerAfter={
            drag !== null && !drag.outside && i === tabs.length - 1 && drag.index === tabs.length
          }
          onActivate={() => {
            activate(doc.id);
          }}
          onPointerDown={(event) => {
            startDrag(doc, event);
          }}
        />
      ))}
      {drag?.outside && (
        <span className="ml-2 shrink-0 rounded-md bg-accent px-2 py-0.5 text-2xs font-medium text-accent-fg">
          Release to open in a new window
        </span>
      )}
    </div>
  );
}

interface TabProps {
  doc: DocumentInfo;
  active: boolean;
  dragging: boolean;
  markerBefore: boolean;
  markerAfter: boolean;
  onActivate: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
}

function Tab({
  doc,
  active,
  dragging,
  markerBefore,
  markerAfter,
  onActivate,
  onPointerDown,
}: TabProps) {
  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const Icon = tool?.icon ?? workspaceInfo(doc.workspace).icon;
  const tip = doc.path ?? (tool ? `${tool.title}, a quick tool` : doc.title);
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>
        <div
          data-doc-tab
          className={`relative flex shrink-0 items-center ${dragging ? "opacity-40" : ""}`}
        >
          {markerBefore && <span className="absolute -left-0.5 h-5 w-0.5 rounded-full bg-accent" />}
          <Tooltip text={tip}>
            <button
              type="button"
              role="tab"
              aria-selected={active}
              onClick={onActivate}
              onPointerDown={onPointerDown}
              onAuxClick={(event) => {
                // middle click closes, like a browser
                if (event.button === 1) void closeTab(doc.id);
              }}
              className={`group flex h-7 max-w-52 items-center gap-1.5 rounded-md pr-1 pl-2.5 text-xs transition-colors duration-(--motion-fast) ${
                active
                  ? "bg-raised text-fg shadow-1"
                  : "text-fg-muted hover:bg-raised/60 hover:text-fg"
              }`}
            >
              <Icon
                className={`size-3.5 shrink-0 ${active ? "text-accent-text" : ""}`}
                strokeWidth={1.8}
              />
              <span className="truncate">{doc.title}</span>
              <span className="relative flex size-4 shrink-0 items-center justify-center">
                {doc.dirty && (
                  <span
                    aria-label="Unsaved changes"
                    className="size-1.5 rounded-full bg-fg-muted group-hover:invisible"
                  />
                )}
                <span
                  role="button"
                  tabIndex={-1}
                  aria-label={`Close ${doc.title}`}
                  onPointerDown={(event) => {
                    event.stopPropagation();
                  }}
                  onClick={(event) => {
                    event.stopPropagation();
                    void closeTab(doc.id);
                  }}
                  className="absolute inset-0 hidden items-center justify-center rounded-sm text-fg-muted group-hover:flex hover:bg-pressed hover:text-fg"
                >
                  <X className="size-3" />
                </span>
              </span>
            </button>
          </Tooltip>
          {markerAfter && <span className="absolute -right-0.5 h-5 w-0.5 rounded-full bg-accent" />}
        </div>
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className={menuContent}>
          <ContextMenu.Item className={menuItem} onSelect={() => void closeTab(doc.id)}>
            Close tab
          </ContextMenu.Item>
          <ContextMenu.Separator className={menuSeparator} />
          <ContextMenu.Item className={menuItem} onSelect={() => void tearOff(doc.id)}>
            Move to a new window
          </ContextMenu.Item>
          {!isMainWindow && (
            <ContextMenu.Item className={menuItem} onSelect={() => void moveToMain(doc.id)}>
              Move to the main window
            </ContextMenu.Item>
          )}
          {doc.path && (
            <>
              <ContextMenu.Separator className={menuSeparator} />
              <ContextMenu.Item
                className={menuItem}
                onSelect={() => {
                  if (doc.path) void navigator.clipboard.writeText(doc.path);
                }}
              >
                Copy path
              </ContextMenu.Item>
            </>
          )}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
