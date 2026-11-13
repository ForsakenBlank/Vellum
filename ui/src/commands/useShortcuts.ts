import { useEffect } from "react";

import { isBareKey, isTyping, shortcutFromEvent } from "../lib/keys";
import { useSession, workspaceOf } from "../stores/session";
import { useApp } from "../stores/app";
import { useUi } from "../stores/ui";
import { allCommands, currentKeymap, type AppCommand } from "./registry";

/** set on the body while the shortcut editor listens for a new key */
export const RECORDING_ATTRIBUTE = "data-recording-shortcut";

function currentScope(): string | null {
  const { session } = useSession.getState();
  const label = useApp.getState().windowLabel;
  const view = session?.windows.find((window) => window.label === label)?.view;
  return view ? workspaceOf(view, session) : null;
}

/** the command a key press should run, if any */
export function commandForKey(event: KeyboardEvent): AppCommand | null {
  const shortcut = shortcutFromEvent(event);
  if (!shortcut) return null;
  if (isTyping(event.target) && isBareKey(shortcut)) return null;

  const ui = useUi.getState();
  const overlayOpen = ui.paletteOpen || ui.settingsPane !== null;
  const keymap = currentKeymap();
  const scope = currentScope();
  return (
    allCommands().find((command) => {
      if (!(keymap[command.id] ?? []).includes(shortcut)) return false;
      if (command.scope !== "global" && command.scope !== scope) return false;
      // while a dialog is up, only the keys that open dialogs still work
      if (overlayOpen && command.id !== "app.palette" && command.id !== "app.settings") {
        return false;
      }
      return command.enabled();
    }) ?? null
  );
}

/** listens for shortcuts anywhere in the window */
export function useGlobalShortcuts(): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || document.body.hasAttribute(RECORDING_ATTRIBUTE)) return;
      const command = commandForKey(event);
      if (!command) return;
      event.preventDefault();
      if (command.id === "app.palette" && useUi.getState().paletteOpen) {
        // the same key closes it again
        useUi.getState().setPaletteOpen(false);
        return;
      }
      void command.run();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, []);
}
