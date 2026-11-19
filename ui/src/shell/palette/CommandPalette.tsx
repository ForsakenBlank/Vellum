import { Dialog as RadixDialog } from "radix-ui";
import {
  Clock,
  CornerDownLeft,
  File,
  LayoutTemplate,
  Search,
  Settings,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Highlight } from "../../components/Highlight";
import { Kbd } from "../../components/Kbd";
import { runCommand, useCommands } from "../../state/commands";
import { useRecent } from "../../state/recent";
import { useSettings } from "../../state/settings";
import { useUi } from "../../state/ui";
import { applyLayout, openPaths } from "../actions";
import { buildItems, searchItems, type PaletteItem, type PaletteSection } from "./items";

const sectionIcons: Record<PaletteSection, LucideIcon> = {
  Recent: Clock,
  Commands: Terminal,
  Files: File,
  Settings: Settings,
  Presets: LayoutTemplate,
};

function perform(item: PaletteItem) {
  const action = item.action;
  switch (action.type) {
    case "command":
      void runCommand(action.kind);
      break;
    case "file":
      void openPaths([action.path]);
      break;
    case "setting":
      useUi.getState().openSettings(action.entry.section, action.entry.label);
      break;
    case "shortcutPreset":
      void useSettings.getState().update({ shortcuts: { preset: action.preset } });
      useUi.getState().toast({ level: "success", title: item.title.replace("Use", "Now using") });
      break;
    case "layout":
      applyLayout(action.name);
      break;
  }
}

/** ctrl+k: fuzzy search over every command, setting, recent file and preset */
export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPalette);
  return (
    <RadixDialog.Root open={open} onOpenChange={setOpen}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="fixed inset-0 z-[var(--z-modal)] animate-fade-in bg-scrim" />
        <RadixDialog.Content
          aria-describedby={undefined}
          className="fixed top-[14vh] left-1/2 z-[var(--z-modal)] flex max-h-[min(30rem,70vh)] w-[min(40rem,calc(100vw-2rem))] -translate-x-1/2 animate-pop-in flex-col overflow-hidden rounded-xl bg-panel shadow-pop"
        >
          <RadixDialog.Title className="sr-only">Command palette</RadixDialog.Title>
          {open && (
            <PaletteBody
              onDone={() => {
                setOpen(false);
              }}
            />
          )}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}

function PaletteBody({ onDone }: { onDone: () => void }) {
  const commands = useCommands((s) => s.infos);
  const keymap = useCommands((s) => s.keymap);
  const recentCommands = useCommands((s) => s.recent);
  const files = useRecent((s) => s.files);
  const savedLayouts = useSettings((s) => s.settings?.layout.saved);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const items = useMemo(
    () =>
      buildItems({
        commands,
        keymap,
        recentCommands,
        files: files ?? [],
        savedLayouts: savedLayouts ?? [],
      }),
    [commands, keymap, recentCommands, files, savedLayouts],
  );
  const results = useMemo(() => searchItems(items, query), [items, query]);

  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-index="${selected}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const choose = (index: number) => {
    const result = results[index];
    if (!result) return;
    onDone();
    perform(result.item);
  };

  let lastSection: PaletteSection | null = null;

  return (
    <>
      <div className="flex items-center gap-2.5 border-b border-line px-4">
        <Search className="size-4 shrink-0 text-fg-subtle" />
        <input
          autoFocus
          role="combobox"
          aria-expanded
          aria-controls="palette-results"
          aria-activedescendant={results[selected] ? `palette-${selected}` : undefined}
          aria-label="Search commands, settings, files and presets"
          placeholder="Search commands, settings, files and presets"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelected(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setSelected((i) => Math.min(results.length - 1, i + 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelected((i) => Math.max(0, i - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              choose(selected);
            }
          }}
          className="h-12 flex-1 bg-transparent text-base text-fg outline-none placeholder:text-fg-subtle focus-visible:shadow-none"
        />
      </div>

      <div
        ref={listRef}
        id="palette-results"
        role="listbox"
        aria-label="Results"
        className="min-h-0 flex-1 overflow-y-auto p-1.5"
      >
        {results.length === 0 ? (
          <p className="px-3 py-8 text-center text-sm text-fg-muted">
            Nothing matches “{query}”. Try fewer letters, or what you want to do, like “gif” or
            “theme”.
          </p>
        ) : (
          results.map(({ item, positions }, index) => {
            const heading = item.section !== lastSection ? item.section : null;
            lastSection = item.section;
            const Icon = sectionIcons[item.section];
            const active = index === selected;
            return (
              <div key={item.id}>
                {heading && (
                  <div className="px-2.5 pt-2 pb-1 text-2xs font-medium tracking-wide text-fg-subtle uppercase">
                    {heading}
                  </div>
                )}
                <div
                  id={`palette-${index}`}
                  data-index={index}
                  role="option"
                  aria-selected={active}
                  onPointerMove={() => {
                    if (!active) setSelected(index);
                  }}
                  onClick={() => {
                    choose(index);
                  }}
                  className={`flex h-10 cursor-default items-center gap-3 rounded-lg px-2.5 ${active ? "bg-elevated" : ""}`}
                >
                  <Icon
                    className={`size-4 shrink-0 ${active ? "text-accent-text" : "text-fg-subtle"}`}
                    strokeWidth={1.7}
                  />
                  <span className="flex min-w-0 flex-1 items-baseline gap-2">
                    <span className="shrink-0 text-sm text-fg">
                      <Highlight text={item.title} positions={positions} />
                    </span>
                    <span className="truncate text-xs text-fg-subtle">{item.subtitle}</span>
                  </span>
                  {item.shortcut && <Kbd chord={item.shortcut} />}
                </div>
              </div>
            );
          })
        )}
      </div>

      <div className="flex items-center gap-4 border-t border-line px-4 py-2 text-2xs text-fg-subtle">
        <span className="flex items-center gap-1.5">
          <kbd className="kbd">↑</kbd>
          <kbd className="kbd">↓</kbd> to move
        </span>
        <span className="flex items-center gap-1.5">
          <kbd className="kbd">
            <CornerDownLeft className="size-2.5" />
          </kbd>
          to run
        </span>
        <span className="flex items-center gap-1.5">
          <kbd className="kbd">Esc</kbd> to close
        </span>
      </div>
    </>
  );
}
