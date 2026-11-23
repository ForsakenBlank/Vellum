import { useEffect, useState } from "react";

import { inDesktopApp } from "../lib/ipc";
import { isMainWindow } from "../lib/window";
import { registerUiHandlers } from "../state/commands";
import { useDocuments } from "../state/documents";
import { useSettings } from "../state/settings";
import { useUi } from "../state/ui";
import { quickTools } from "../workspaces/quickTools";
import { ImageShell } from "../workspaces/image/ImageShell";
import { WorkspaceView } from "../workspaces/WorkspaceView";
import { workspaces } from "../workspaces/workspaces";
import {
  applyLayout,
  closeTab,
  cycleTab,
  flipDocks,
  flipPanel,
  moveToMain,
  openTool,
  pickAndOpen,
  tearOff,
} from "./actions";
import { ConfirmDialog } from "./ConfirmDialog";
import { DockFrame } from "./Dock";
import { DropOverlay } from "./DropOverlay";
import { Home } from "./Home";
import { CommandPalette } from "./palette/CommandPalette";
import { panels } from "./panels/registry";
import { Rail } from "./Rail";
import { RecoveryPrompt } from "./RecoveryPrompt";
import { SaveLayoutDialog } from "./SaveLayoutDialog";
import { SettingsDialog } from "./settings/SettingsDialog";
import { StatusBar } from "./StatusBar";
import { TitleBar } from "./TitleBar";
import { Toasts } from "./Toasts";
import { useShortcuts } from "./useShortcuts";

export function AppShell() {
  const [savingLayout, setSavingLayout] = useState(false);
  const settingsError = useSettings((s) => s.error);
  const showHome = useDocuments((s) => s.activeId === null && s.emptyWorkspace === null);
  useShortcuts();

  // what the ui actions handed back by app commands do in this window
  useEffect(() => {
    const ui = useUi.getState();
    const docs = useDocuments.getState();
    const handlers: Record<string, () => void | Promise<void>> = {
      "app.command_palette": () => {
        ui.setPalette(!useUi.getState().paletteOpen);
      },
      "app.settings": () => {
        ui.openSettings("appearance");
      },
      "app.shortcuts": () => {
        ui.openSettings("shortcuts");
      },
      "app.notifications": () => {
        ui.setNotificationsOpen(true);
      },
      "file.open": () => pickAndOpen(),
      "file.close": () => closeTab(),
      "window.next_tab": () => {
        cycleTab(1);
      },
      "window.previous_tab": () => {
        cycleTab(-1);
      },
      "window.tear_off": () => tearOff(),
      "window.move_to_main": () => moveToMain(),
    };
    if (isMainWindow) {
      Object.assign(handlers, {
        "app.home": () => {
          docs.showHome();
        },
        "view.docks.toggle": flipDocks,
        "view.layout.save": () => {
          setSavingLayout(true);
        },
        "view.layout.reset": () => {
          applyLayout("default");
        },
      });
      for (const panel of panels) {
        handlers[panel.command] = () => {
          flipPanel(panel.id);
        };
      }
      for (const ws of workspaces) {
        handlers[ws.command] = () => {
          docs.showWorkspace(ws.id);
        };
      }
      for (const tool of quickTools) {
        handlers[tool.command] = () => openTool(tool.id);
      }
    }
    return registerUiHandlers(handlers);
  }, []);

  const center = isMainWindow && showHome ? <Home /> : <WorkspaceView />;

  return (
    <div className="flex h-full flex-col bg-canvas">
      <TitleBar />
      {!inDesktopApp && (
        <p
          role="status"
          className="shrink-0 border-b border-line bg-warning/10 px-4 py-1.5 text-xs text-fg-muted"
        >
          This page is running in a browser, so nothing reaches the Rust side. Start the desktop app
          with <code className="font-mono">pnpm tauri dev</code> to open files and run jobs.
        </p>
      )}
      {settingsError && (
        <p
          role="alert"
          className="shrink-0 border-b border-line bg-danger/10 px-4 py-1.5 text-xs text-danger"
        >
          Settings could not be saved: {settingsError}. Try again, or restart Vellum if it keeps
          happening.
        </p>
      )}
      <div className="flex min-h-0 flex-1">
        {isMainWindow && <Rail />}
        <main className="flex min-h-0 min-w-0 flex-1 bg-canvas">
          {isMainWindow ? (
            <DockFrame>{center}</DockFrame>
          ) : (
            <div className="min-w-0 flex-1">{center}</div>
          )}
        </main>
      </div>
      <StatusBar
        onSaveLayout={() => {
          setSavingLayout(true);
        }}
      />

      <CommandPalette />
      <SettingsDialog />
      <SaveLayoutDialog
        open={savingLayout}
        onClose={() => {
          setSavingLayout(false);
        }}
      />
      <ConfirmDialog />
      {isMainWindow && <RecoveryPrompt />}
      <Toasts />
      <ImageShell />
      <DropOverlay />
    </div>
  );
}
