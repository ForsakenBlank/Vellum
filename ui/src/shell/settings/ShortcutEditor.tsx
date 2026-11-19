import { Plus, RotateCcw, CircleExclamationPoint, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type { ShortcutPreset } from "../../bindings/ShortcutPreset";
import { Button, IconButton } from "../../components/Button";
import { Kbd } from "../../components/Kbd";
import { rank } from "../../lib/fuzzy";
import { chordFromEvent } from "../../lib/keys";
import { commandInfo, useCommands } from "../../state/commands";
import { useSettings } from "../../state/settings";
import { shortcutPresetNames } from "../palette/items";
import { addChord, clashesFor, removeChord, resetCommand } from "./shortcutEdits";

interface Pending {
  kind: string;
  chord: string;
  clashes: string[];
}

/** every command with its shortcuts, a preset picker and live conflict checks */
export function ShortcutEditor({ query }: { query: string }) {
  const infos = useCommands((s) => s.infos);
  const keymap = useCommands((s) => s.keymap);
  const shortcuts = useSettings((s) => s.settings?.shortcuts);
  const update = useSettings((s) => s.update);
  const [recording, setRecording] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);

  const bindings = useMemo(() => keymap?.bindings ?? {}, [keymap]);
  const overrides = useMemo(() => shortcuts?.overrides ?? {}, [shortcuts]);

  const save = (next: Record<string, string[]>) => {
    // settings changes are json merge patches, where null is how a key gets removed
    const patch: Record<string, string[] | null> = { ...next };
    for (const kind of Object.keys(overrides)) if (!(kind in next)) patch[kind] = null;
    void update({ shortcuts: { overrides: patch as Record<string, string[]> } });
  };

  // while recording, the next key press becomes the chord. it is caught before the app's
  // own shortcut handler so pressing Ctrl+K here does not open the palette.
  useEffect(() => {
    if (!recording) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.code === "Escape" && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        setRecording(null);
        return;
      }
      const chord = chordFromEvent(event);
      if (!chord) return;
      setRecording(null);
      const clashes = clashesFor(bindings, recording, chord);
      if (clashes.length > 0) {
        setPending({ kind: recording, chord, clashes });
      } else {
        save(addChord(bindings, overrides, recording, chord, false));
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
    };
    // save is rebuilt each render, the values it reads are in the list
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording, bindings, overrides]);

  const groups = useMemo(() => {
    const matched = rank(query, infos, (info) => ({
      title: info.title,
      keywords: `${info.category} ${info.description} ${(bindings[info.kind] ?? []).join(" ")}`,
    })).map((r) => r.item);
    const byCategory = new Map<string, typeof matched>();
    for (const info of matched) {
      byCategory.set(info.category, [...(byCategory.get(info.category) ?? []), info]);
    }
    return [...byCategory.entries()];
  }, [infos, bindings, query]);

  const setPreset = (preset: ShortcutPreset) => {
    void update({ shortcuts: { preset } });
  };

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <span className="text-sm font-medium">Start from</span>
        <div role="radiogroup" aria-label="Shortcut preset" className="flex flex-wrap gap-1.5">
          {(Object.keys(shortcutPresetNames) as ShortcutPreset[]).map((preset) => {
            const on = shortcuts?.preset === preset;
            return (
              <button
                key={preset}
                type="button"
                role="radio"
                aria-checked={on}
                onClick={() => {
                  setPreset(preset);
                }}
                className={`h-(--row) rounded-md border px-3 text-xs transition-colors duration-(--motion-fast) ${
                  on
                    ? "border-accent bg-accent-soft text-fg"
                    : "border-line-strong text-fg-muted hover:bg-elevated hover:text-fg"
                }`}
              >
                {shortcutPresetNames[preset]}
              </button>
            );
          })}
        </div>
        <p className="text-xs text-fg-subtle">
          Your own changes stay on top of whichever preset you pick.
          {Object.keys(overrides).length > 0 && (
            <>
              {" "}
              <button
                type="button"
                className="text-accent-text hover:underline"
                onClick={() => {
                  save({});
                }}
              >
                Reset all {Object.keys(overrides).length} changes
              </button>
            </>
          )}
        </p>
      </div>

      {keymap && keymap.conflicts.length > 0 && (
        <div
          role="alert"
          className="flex flex-col gap-1 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs"
        >
          <span className="flex items-center gap-1.5 font-medium text-warning">
            <CircleExclamationPoint className="size-3.5" />
            {keymap.conflicts.length === 1
              ? "One shortcut does two things"
              : `${keymap.conflicts.length} shortcuts do two things`}
          </span>
          {keymap.conflicts.map((conflict) => (
            <span key={conflict.chord} className="flex items-center gap-2 text-fg-muted">
              <Kbd chord={conflict.chord} />
              {conflict.commands.map((k) => commandInfo(k)?.title ?? k).join(" and ")}
            </span>
          ))}
        </div>
      )}

      {groups.length === 0 && <p className="text-sm text-fg-muted">No commands match “{query}”.</p>}

      {groups.map(([category, list]) => (
        <section key={category} className="flex flex-col">
          <h4 className="pb-1 text-2xs font-medium tracking-wide text-fg-subtle uppercase">
            {category}
          </h4>
          <ul className="divide-y divide-line">
            {list.map((info) => {
              const chords = bindings[info.kind] ?? [];
              const changed = info.kind in overrides;
              const isRecording = recording === info.kind;
              const isPending = pending?.kind === info.kind;
              return (
                <li key={info.kind} className="flex flex-col gap-1.5 py-1.5">
                  <div className="flex min-h-(--row) items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm" title={info.description}>
                      {info.title}
                      {changed && <span className="ml-2 text-2xs text-accent-text">Changed</span>}
                    </span>
                    {chords.map((chord) => (
                      <span
                        key={chord}
                        className="group/chord flex items-center gap-0.5 rounded-md bg-raised py-0.5 pr-0.5 pl-1.5"
                      >
                        <Kbd chord={chord} />
                        <IconButton
                          size="sm"
                          label={`Remove ${chord} from ${info.title}`}
                          icon={<X className="size-3" />}
                          className="size-5"
                          onClick={() => {
                            save(removeChord(bindings, overrides, info.kind, chord));
                          }}
                        />
                      </span>
                    ))}
                    {isRecording ? (
                      <span className="animate-pulse rounded-md border border-accent px-2 py-0.5 text-xs text-accent-text">
                        Press keys, Esc to stop
                      </span>
                    ) : (
                      <IconButton
                        size="sm"
                        label={`Add a shortcut for ${info.title}`}
                        icon={<Plus className="size-3.5" />}
                        onClick={() => {
                          setPending(null);
                          setRecording(info.kind);
                        }}
                      />
                    )}
                    <IconButton
                      size="sm"
                      label={`Reset ${info.title} to the preset`}
                      icon={<RotateCcw className="size-3.5" />}
                      disabled={!changed}
                      onClick={() => {
                        save(resetCommand(overrides, info.kind));
                      }}
                    />
                  </div>
                  {isPending && (
                    <div
                      role="alert"
                      className="flex items-center gap-2 rounded-md bg-warning/10 px-2 py-1.5 text-xs"
                    >
                      <Kbd chord={pending.chord} />
                      <span className="flex-1 text-fg-muted">
                        is already used by{" "}
                        {pending.clashes.map((k) => commandInfo(k)?.title ?? k).join(" and ")}.
                      </span>
                      <Button
                        size="sm"
                        variant="primary"
                        tooltip="Move the shortcut to this command"
                        onClick={() => {
                          save(addChord(bindings, overrides, pending.kind, pending.chord, true));
                          setPending(null);
                        }}
                      >
                        Use it here
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        tooltip="Keep shortcuts as they were"
                        onClick={() => {
                          setPending(null);
                        }}
                      >
                        Cancel
                      </Button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}
