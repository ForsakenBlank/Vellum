import { clsx } from "clsx";
import {
  Clock,
  CornerDownLeft,
  FileQuestion,
  Keyboard,
  LayoutPanelLeft,
  Search,
  Settings2,
  SquareTerminal,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";

import { allCommands, useKeymap } from "../commands/registry";
import { PRESET_ORDER, PRESETS } from "../commands/shortcuts";
import { Dialog } from "../components/Dialog";
import { Kbd } from "../components/Kbd";
import { mediaIcon } from "../components/mediaIcon";
import { savedLayouts, saveLayout } from "../dock/state";
import { fuzzyRank } from "../lib/fuzzy";
import { useRecent } from "../stores/recent";
import { openFiles, useWorkspaceId } from "../stores/session";
import { updateSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { workspaceById } from "../workspaces";
import { PANES, SETTINGS } from "./settings/catalog";

type Group = "Recently used" | "Files" | "Commands" | "Settings" | "Presets";

export interface PaletteItem {
  key: string;
  label: string;
  group: Group;
  keywords: string[];
  icon: LucideIcon;
  /** a quiet second line or trailing text, such as a folder or a category */
  hint?: string;
  /** the command behind the item, to show its keys */
  commandId?: string;
  run: () => unknown;
}

const MRU_KEY = "vellum.palette.recent";
const MRU_SIZE = 6;
const MAX_RESULTS = 60;

function readRecentlyUsed(): string[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(MRU_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((key) => typeof key === "string") : [];
  } catch {
    return [];
  }
}

function rememberUsed(key: string) {
  const next = [key, ...readRecentlyUsed().filter((other) => other !== key)].slice(0, MRU_SIZE);
  try {
    localStorage.setItem(MRU_KEY, JSON.stringify(next));
  } catch {
    // a full or blocked storage only costs the "recently used" list
  }
}

function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut) : path;
}

/** everything the palette can find, built fresh each time it opens */
function usePaletteItems(open: boolean): PaletteItem[] {
  const recent = useRecent((state) => state.entries);
  const layouts = useSettings((state) => state.sections?.layouts);
  const workspaceId = useWorkspaceId();
  const ui = useUi.getState;

  return useMemo(() => {
    if (!open) return [];
    const items: PaletteItem[] = [];

    for (const entry of recent ?? []) {
      if (!entry.exists) continue;
      items.push({
        key: `file:${entry.path}`,
        label: entry.name,
        group: "Files",
        keywords: [entry.path],
        icon: mediaIcon(entry.kind),
        hint: folderOf(entry.path),
        run: () => openFiles([entry.path]),
      });
    }

    for (const command of allCommands()) {
      if (!command.palette || !command.enabled()) continue;
      items.push({
        key: `command:${command.id}`,
        label: command.title,
        group: "Commands",
        keywords: [...command.keywords, command.category],
        icon: SquareTerminal,
        hint: command.category,
        commandId: command.id,
        run: command.run,
      });
    }

    for (const setting of SETTINGS) {
      const pane = PANES.find((entry) => entry.id === setting.pane)?.title ?? "";
      items.push({
        key: `setting:${setting.id}`,
        label: setting.title,
        group: "Settings",
        keywords: [...setting.keywords, setting.description],
        icon: Settings2,
        hint: `Settings, ${pane}`,
        run: () => {
          ui().openSettings(setting.pane, setting.id);
        },
      });
    }

    for (const preset of PRESET_ORDER) {
      items.push({
        key: `preset:shortcuts:${preset}`,
        label: `Use ${PRESETS[preset].label} shortcuts`,
        group: "Presets",
        keywords: ["shortcut preset", "keymap", "keys"],
        icon: Keyboard,
        hint: "Shortcut preset",
        run: () => updateSettings("shortcuts", { preset }),
      });
    }

    const workspace = workspaceById(workspaceId);
    for (const saved of savedLayouts(workspace, layouts)) {
      items.push({
        key: `preset:layout:${workspace.id}:${saved.name}`,
        label: `Switch to the ${saved.name} layout`,
        group: "Presets",
        keywords: ["panel layout", "workspace layout", saved.name],
        icon: LayoutPanelLeft,
        hint: `${workspace.title} layout`,
        run: () => saveLayout(workspace, saved.layout),
      });
    }
    return items;
  }, [open, recent, layouts, workspaceId, ui]);
}

/** the items shown before anything is typed: what was used lately, then files */
function startingItems(items: PaletteItem[]): PaletteItem[] {
  const byKey = new Map(items.map((item) => [item.key, item]));
  const used = readRecentlyUsed()
    .map((key) => byKey.get(key))
    .filter((item): item is PaletteItem => item !== undefined)
    .map((item) => ({ ...item, group: "Recently used" as const }));
  const usedKeys = new Set(used.map((item) => item.key));
  const files = items.filter((item) => item.group === "Files" && !usedKeys.has(item.key));
  const commands = items.filter((item) => item.group === "Commands" && !usedKeys.has(item.key));
  return [...used, ...files.slice(0, 5), ...commands];
}

function Highlighted({ text, positions }: { text: string; positions: number[] }) {
  if (positions.length === 0) return <>{text}</>;
  const marks = new Set(positions);
  const parts: ReactNode[] = [];
  let run = "";
  let marked = false;
  const flush = (index: number) => {
    if (!run) return;
    parts.push(
      marked ? (
        <mark key={index} className="bg-transparent font-semibold text-accent">
          {run}
        </mark>
      ) : (
        run
      ),
    );
    run = "";
  };
  // indices are utf-16 positions, the same ones the matcher reports
  for (let index = 0; index < text.length; index++) {
    if (marks.has(index) !== marked) {
      flush(index);
      marked = !marked;
    }
    run += text.charAt(index);
  }
  flush(text.length);
  return <>{parts}</>;
}

/** the command palette: one search box for every command, setting, file and preset */
export function Palette() {
  const open = useUi((state) => state.paletteOpen);
  const setOpen = useUi((state) => state.setPaletteOpen);

  return (
    <Dialog
      open={open}
      onOpenChange={setOpen}
      title="Command palette"
      description="Search for commands, settings, recent files and presets."
      bare
      className="top-[14vh] w-[min(38rem,calc(100vw-2rem))]"
    >
      {open && (
        <PaletteBody
          close={() => {
            setOpen(false);
          }}
        />
      )}
    </Dialog>
  );
}

function PaletteBody({ close }: { close: () => void }) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const items = usePaletteItems(true);
  const keymap = useKeymap();
  const listId = useId();
  const listRef = useRef<HTMLDivElement>(null);

  const results = useMemo(() => {
    if (!query.trim()) {
      return startingItems(items).map((item) => ({ item, positions: [] as number[] }));
    }
    return fuzzyRank(query, items).slice(0, MAX_RESULTS);
  }, [query, items]);

  const current = Math.min(selected, Math.max(0, results.length - 1));

  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-index="${current}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const choose = (item: PaletteItem) => {
    rememberUsed(item.key);
    close();
    void item.run();
  };

  const optionId = (index: number) => `${listId}-${index}`;

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex items-center gap-3 border-b border-line px-4">
        <Search className="size-4 shrink-0 text-ink-soft" aria-hidden />
        <input
          autoFocus
          role="combobox"
          aria-expanded
          aria-controls={listId}
          aria-activedescendant={results.length > 0 ? optionId(current) : undefined}
          aria-label="Search commands, settings and files"
          placeholder="Type a command, a setting or a file name"
          spellCheck={false}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelected(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setSelected((current + 1) % Math.max(1, results.length));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelected((current - 1 + results.length) % Math.max(1, results.length));
            } else if (event.key === "Enter") {
              event.preventDefault();
              const result = results[current];
              if (result) choose(result.item);
            }
          }}
          className="h-12 min-w-0 flex-1 bg-transparent text-[0.9375rem] text-ink outline-none placeholder:text-ink-faint"
        />
      </div>

      <div
        ref={listRef}
        id={listId}
        role="listbox"
        aria-label="Results"
        className="max-h-[min(26rem,55vh)] overflow-y-auto p-1.5"
      >
        {results.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-4 py-10 text-center text-sm text-ink-soft">
            <FileQuestion className="size-5" aria-hidden />
            Nothing matches “{query}”.
          </div>
        ) : (
          results.map(({ item, positions }, index) => {
            const previous = results[index - 1]?.item.group;
            const showGroup = !query.trim() && previous !== item.group;
            const Icon = item.group === "Recently used" ? Clock : item.icon;
            const keys = item.commandId ? keymap[item.commandId] : undefined;
            return (
              <div key={item.key}>
                {showGroup && (
                  <div className="px-2.5 pt-2.5 pb-1 text-2xs font-medium tracking-wide text-ink-faint uppercase">
                    {item.group}
                  </div>
                )}
                <div
                  id={optionId(index)}
                  role="option"
                  aria-selected={index === current}
                  data-index={index}
                  onPointerMove={() => {
                    if (index !== current) setSelected(index);
                  }}
                  onClick={() => {
                    choose(item);
                  }}
                  className={clsx(
                    "flex h-9 items-center gap-3 rounded-md px-2.5 text-sm",
                    index === current ? "bg-hover text-ink" : "text-ink-soft",
                  )}
                >
                  <Icon className="size-4 shrink-0 text-ink-soft" aria-hidden />
                  <span className="min-w-0 truncate text-ink">
                    <Highlighted text={item.label} positions={positions} />
                  </span>
                  {item.hint && (
                    <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                      {item.hint}
                    </span>
                  )}
                  <span className="ml-auto flex shrink-0 items-center gap-1">
                    {keys?.[0] && <Kbd shortcut={keys[0]} />}
                    {index === current && (
                      <CornerDownLeft className="size-3.5 text-ink-faint" aria-hidden />
                    )}
                  </span>
                </div>
              </div>
            );
          })
        )}
      </div>

      <footer className="flex items-center gap-4 border-t border-line px-4 py-2 text-2xs text-ink-faint">
        <span className="flex items-center gap-1.5">
          <Kbd shortcut="ArrowUp" />
          <Kbd shortcut="ArrowDown" /> to move
        </span>
        <span className="flex items-center gap-1.5">
          <Kbd shortcut="Enter" /> to run
        </span>
        <span className="flex items-center gap-1.5">
          <Kbd shortcut="Escape" /> to close
        </span>
      </footer>
    </div>
  );
}
