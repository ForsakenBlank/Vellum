import { clsx } from "clsx";
import { Plus, RotateCcw, Search, TriangleAlert, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { bindableCommands, type AppCommand } from "../../commands/registry";
import {
  assignShortcut,
  buildKeymap,
  clashesWith,
  findConflicts,
  removeShortcut,
  resetShortcut,
} from "../../commands/shortcuts";
import { RECORDING_ATTRIBUTE } from "../../commands/useShortcuts";
import { Button, IconButton } from "../../components/Button";
import { Kbd } from "../../components/Kbd";
import { fuzzyRank } from "../../lib/fuzzy";
import { formatShortcut, shortcutFromEvent } from "../../lib/keys";
import { useBackendCommands } from "../../stores/commands";
import { updateSettings, useSettings } from "../../stores/settings";
import { workspaceById } from "../../workspaces";

/** a key the user pressed that another command already uses */
interface PendingClash {
  command: AppCommand;
  shortcut: string;
  clashes: AppCommand[];
}

/** listens for the next key combination while `active` */
function useRecorder(active: boolean, onKeys: (shortcut: string) => void, onCancel: () => void) {
  const handlers = useRef({ onKeys, onCancel });
  useEffect(() => {
    handlers.current = { onKeys, onCancel };
  });
  useEffect(() => {
    if (!active) return;
    document.body.setAttribute(RECORDING_ATTRIBUTE, "");
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const plain = !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey;
      if (event.key === "Escape" && plain) {
        handlers.current.onCancel();
        return;
      }
      const shortcut = shortcutFromEvent(event);
      if (shortcut) handlers.current.onKeys(shortcut);
    };
    // capture, so the dialog does not close on escape and nothing else reacts
    window.addEventListener("keydown", onKey, true);
    return () => {
      document.body.removeAttribute(RECORDING_ATTRIBUTE);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [active]);
}

export function ShortcutEditor({ presetRow }: { presetRow: ReactNode }) {
  const settings = useSettings((state) => state.sections?.shortcuts);
  const backend = useBackendCommands((state) => state.backend);
  const commands = useMemo(() => bindableCommands(backend), [backend]);
  const keymap = useMemo(() => buildKeymap(commands, settings ?? null), [commands, settings]);
  const conflicts = useMemo(() => findConflicts(commands, keymap), [commands, keymap]);
  const [filter, setFilter] = useState("");
  const [onlyConflicts, setOnlyConflicts] = useState(false);
  const [recording, setRecording] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingClash | null>(null);

  const conflicted = useMemo(
    () => new Set(conflicts.flatMap((conflict) => conflict.commands.map((c) => c.id))),
    [conflicts],
  );

  const save = (bindings: Record<string, string[]>) =>
    void updateSettings("shortcuts", { bindings });

  const recordingCommand = commands.find((command) => command.id === recording) ?? null;
  const onKeys = (shortcut: string) => {
    if (!recordingCommand || !settings) return;
    setRecording(null);
    const clashes = clashesWith(shortcut, recordingCommand, commands, keymap);
    if (clashes.length > 0) {
      setPending({ command: recordingCommand, shortcut, clashes });
      return;
    }
    save(assignShortcut(settings, commands, recordingCommand, shortcut, false));
  };
  const onCancel = () => {
    setRecording(null);
  };
  useRecorder(recording !== null, onKeys, onCancel);

  const visible = useMemo(() => {
    let list = commands;
    if (onlyConflicts) list = list.filter((command) => conflicted.has(command.id));
    if (!filter.trim()) return list;
    // typing a key combination finds the commands using it too
    const byKey = list.filter((command) =>
      (keymap[command.id] ?? []).some((key) =>
        formatShortcut(key).toLowerCase().includes(filter.trim().toLowerCase()),
      ),
    );
    const byName = fuzzyRank(
      filter,
      list.map((command) => ({ command, label: command.title, keywords: command.keywords })),
    ).map((result) => result.item.command);
    return [...new Set([...byName, ...byKey])];
  }, [commands, onlyConflicts, conflicted, filter, keymap]);

  const groups = useMemo(() => {
    const byCategory = new Map<string, AppCommand[]>();
    for (const command of visible) {
      byCategory.set(command.category, [...(byCategory.get(command.category) ?? []), command]);
    }
    return [...byCategory];
  }, [visible]);

  if (!settings) return null;

  return (
    <section aria-label="Shortcuts">
      <h2 className="px-3 pt-1 pb-2 text-lg font-semibold">Shortcuts</h2>
      {presetRow}

      <div className="sticky top-0 z-10 mt-2 flex items-center gap-2 bg-surface px-3 py-2">
        <label className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md border border-line bg-canvas px-2.5 focus-within:border-accent">
          <Search className="size-3.5 shrink-0 text-ink-faint" aria-hidden />
          <input
            aria-label="Filter shortcuts"
            placeholder="Filter by name or key"
            value={filter}
            onChange={(event) => {
              setFilter(event.target.value);
            }}
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint"
          />
        </label>
        {conflicts.length > 0 && (
          <Button
            tip="Only show commands that share a key"
            aria-pressed={onlyConflicts}
            className={clsx(onlyConflicts && "border-warn text-warn")}
            onClick={() => {
              setOnlyConflicts(!onlyConflicts);
            }}
          >
            <TriangleAlert className="size-3.5 text-warn" aria-hidden />
            {conflicts.length === 1 ? "1 conflict" : `${conflicts.length} conflicts`}
          </Button>
        )}
        <Button
          tip="Forget every change you made and use the preset as it is"
          disabled={Object.keys(settings.bindings).length === 0}
          onClick={() => {
            save({});
          }}
        >
          Reset all
        </Button>
      </div>

      {pending && (
        <div
          role="alertdialog"
          aria-label="Shortcut already in use"
          className="mx-3 my-2 flex flex-wrap items-center gap-3 rounded-lg border border-warn/50 bg-warn/10 px-3 py-2.5 text-sm"
        >
          <TriangleAlert className="size-4 shrink-0 text-warn" aria-hidden />
          <p className="min-w-0 flex-1">
            <Kbd shortcut={pending.shortcut} /> is already used by{" "}
            <strong>{pending.clashes.map((clash) => clash.title).join(", ")}</strong>.
          </p>
          <Button
            variant="primary"
            onClick={() => {
              save(assignShortcut(settings, commands, pending.command, pending.shortcut, true));
              setPending(null);
            }}
          >
            Use it for {pending.command.title}
          </Button>
          <Button
            tip="Both commands keep the key. The first one listed wins until you change one"
            onClick={() => {
              save(assignShortcut(settings, commands, pending.command, pending.shortcut, false));
              setPending(null);
            }}
          >
            Keep both
          </Button>
          <Button
            onClick={() => {
              setPending(null);
            }}
          >
            Cancel
          </Button>
        </div>
      )}

      {groups.length === 0 && (
        <p className="px-3 py-6 text-sm text-ink-soft">No command matches that.</p>
      )}
      {groups.map(([category, list]) => (
        <div key={category} className="mt-3">
          <h3 className="px-3 pb-1 text-2xs font-medium tracking-wide text-ink-faint uppercase">
            {category}
          </h3>
          <ul className="divide-y divide-line">
            {list.map((command) => {
              const keys = keymap[command.id] ?? [];
              const changed = command.id in settings.bindings;
              const clashing = conflicted.has(command.id);
              return (
                <li
                  key={command.id}
                  aria-label={command.title}
                  className="flex min-h-10 items-center gap-3 px-3 py-1.5"
                >
                  <span className="min-w-0 flex-1 truncate text-sm">
                    {command.title}
                    {command.scope !== "global" && (
                      <span className="ml-2 text-xs text-ink-faint">
                        in {workspaceById(command.scope).title}
                      </span>
                    )}
                  </span>
                  {clashing && (
                    <TriangleAlert
                      className="size-3.5 shrink-0 text-warn"
                      aria-label="Shares a key with another command"
                    />
                  )}
                  <span className="flex shrink-0 flex-wrap items-center justify-end gap-1">
                    {keys.map((key) => (
                      <span key={key} className="group inline-flex items-center">
                        <Kbd shortcut={key} />
                        <button
                          type="button"
                          aria-label={`Remove ${formatShortcut(key)} from ${command.title}`}
                          onClick={() => {
                            save(removeShortcut(settings, commands, command, key));
                          }}
                          className="v-focus ml-0.5 hidden size-4 items-center justify-center rounded text-ink-faint group-hover:inline-flex hover:text-bad focus-visible:inline-flex"
                        >
                          <X className="size-3" aria-hidden />
                        </button>
                      </span>
                    ))}
                    {recording === command.id ? (
                      <span
                        role="status"
                        className="inline-flex h-6 items-center rounded-md border border-accent bg-accent-soft px-2 text-xs text-ink"
                      >
                        Press keys, Esc to cancel
                      </span>
                    ) : (
                      <IconButton
                        label={`Add a shortcut to ${command.title}`}
                        onClick={() => {
                          setPending(null);
                          setRecording(command.id);
                        }}
                      >
                        <Plus className="size-3.5" />
                      </IconButton>
                    )}
                    <IconButton
                      label={`Reset ${command.title} to the preset`}
                      disabled={!changed}
                      className={clsx(!changed && "invisible")}
                      onClick={() => {
                        save(resetShortcut(settings, command));
                      }}
                    >
                      <RotateCcw className="size-3.5" />
                    </IconButton>
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </section>
  );
}
