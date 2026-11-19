import { useEffect } from "react";

import { commandsForChord, chordFromEvent, firesWhileTyping, isEditable } from "../lib/keys";
import { runCommand, useCommands } from "../state/commands";
import { activeDocument, useDocuments } from "../state/documents";
import { contextOf } from "./settings/shortcutEdits";

/** the command a key press should run, if any. exported for tests. */
export function commandForKey(
  event: KeyboardEvent,
  bindings: Partial<Record<string, string[]>>,
  workspace: string | null,
): string | null {
  const chord = chordFromEvent(event);
  if (!chord) return null;
  if (isEditable(event.target) && !firesWhileTyping(chord)) return null;
  const context = workspace?.replace("-", "_") ?? null;
  const kinds = commandsForChord(bindings, chord).filter((kind) => {
    const own = contextOf(kind);
    return own === "global" || own === context;
  });
  // a workspace's own command wins over a global one on the same keys
  return kinds.find((k) => contextOf(k) !== "global") ?? kinds[0] ?? null;
}

/** runs app commands from the keymap rust sends */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat) return;
      const bindings = useCommands.getState().keymap?.bindings;
      if (!bindings) return;
      const workspace = activeDocument(useDocuments.getState())?.workspace ?? null;
      const kind = commandForKey(event, bindings, workspace);
      if (!kind) return;
      event.preventDefault();
      void runCommand(kind);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, []);
}
