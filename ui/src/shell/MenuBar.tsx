import { Menubar } from "radix-ui";
import { Check, ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

import { Kbd } from "../components/Kbd";
import { menuContent, menuItem, menuLabel, menuSeparator } from "../components/menuStyles";
import { isPanelVisible, layoutPresets } from "../lib/dock";
import { fileName } from "../lib/routing";
import { isMainWindow } from "../lib/window";
import { runCommand, useCommands } from "../state/commands";
import { activeDocument, useDocuments } from "../state/documents";
import { useRecent } from "../state/recent";
import { useSettings } from "../state/settings";
import { workspaces } from "../workspaces/workspaces";
import { applyLayout, currentLayout, openPaths } from "./actions";
import { panels } from "./panels/registry";

const trigger =
  "flex h-7 items-center rounded-md px-2 text-xs text-fg-muted outline-none select-none data-[highlighted]:bg-raised data-[state=open]:bg-raised data-[state=open]:text-fg hover:text-fg";

interface ItemProps {
  kind: string;
  /** replaces the command title, eg "Undo Rename" */
  label?: string | undefined;
  disabled?: boolean;
  checked?: boolean;
}

/** a menu item that runs an app command and shows its current shortcut */
function Item({ kind, label, disabled = false, checked }: ItemProps) {
  const title = useCommands((s) => s.infos.find((i) => i.kind === kind)?.title ?? kind);
  const chord = useCommands((s) => s.keymap?.bindings[kind]?.[0]);
  return (
    <Menubar.Item className={menuItem} disabled={disabled} onSelect={() => void runCommand(kind)}>
      <span className="flex w-3.5 justify-center">{checked && <Check className="size-3.5" />}</span>
      <span className="flex-1 truncate">{label ?? title}</span>
      {chord && <Kbd chord={chord} className="opacity-80" />}
    </Menubar.Item>
  );
}

function Sub({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Menubar.Sub>
      <Menubar.SubTrigger className={`${menuItem} data-[state=open]:bg-pressed`}>
        <span className="w-3.5" />
        <span className="flex-1">{label}</span>
        <ChevronRight className="size-3.5 text-fg-subtle" />
      </Menubar.SubTrigger>
      <Menubar.Portal>
        <Menubar.SubContent className={menuContent} sideOffset={4}>
          {children}
        </Menubar.SubContent>
      </Menubar.Portal>
    </Menubar.Sub>
  );
}

function Menu({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Menubar.Menu>
      <Menubar.Trigger className={trigger}>{label}</Menubar.Trigger>
      <Menubar.Portal>
        <Menubar.Content className={menuContent} align="start" sideOffset={6}>
          {children}
        </Menubar.Content>
      </Menubar.Portal>
    </Menubar.Menu>
  );
}

const Separator = () => <Menubar.Separator className={menuSeparator} />;

export function MenuBar() {
  const doc = useDocuments(activeDocument);
  const summary = useDocuments((s) => (s.activeId ? s.summaries[s.activeId] : undefined));
  const recent = useRecent((s) => s.files);
  const settings = useSettings((s) => s.settings);
  const look = settings?.appearance;
  const layout = currentLayout(settings);

  return (
    <Menubar.Root aria-label="Menu" className="flex items-center gap-0.5">
      <Menu label="File">
        <Item kind="file.open" />
        {isMainWindow && (
          <Sub label="Open recent">
            {recent && recent.length > 0 ? (
              <>
                {recent.slice(0, 10).map((file) => (
                  <Menubar.Item
                    key={file.path}
                    className={menuItem}
                    onSelect={() => void openPaths([file.path])}
                  >
                    <span className="w-3.5" />
                    <span className="max-w-72 flex-1 truncate">{fileName(file.path)}</span>
                  </Menubar.Item>
                ))}
                <Separator />
                <Item kind="recent.clear" />
              </>
            ) : (
              <Menubar.Item className={menuItem} disabled>
                <span className="w-3.5" />
                No recent files yet
              </Menubar.Item>
            )}
          </Sub>
        )}
        <Separator />
        <Item kind="file.save" disabled={!doc} />
        <Item kind="file.close" disabled={!doc} />
      </Menu>

      <Menu label="Edit">
        <Item
          kind="edit.undo"
          label={summary?.undoName ? `Undo ${summary.undoName}` : undefined}
          disabled={!summary?.canUndo}
        />
        <Item
          kind="edit.redo"
          label={summary?.redoName ? `Redo ${summary.redoName}` : undefined}
          disabled={!summary?.canRedo}
        />
        <Separator />
        <Item kind="app.command_palette" />
        <Item kind="app.settings" />
        <Item kind="app.shortcuts" />
      </Menu>

      <Menu label="View">
        <Sub label="Theme">
          <Item kind="view.theme.dark" checked={look?.theme === "dark"} />
          <Item kind="view.theme.light" checked={look?.theme === "light"} />
          <Item kind="view.theme.high_contrast" checked={look?.theme === "high-contrast"} />
        </Sub>
        <Item kind="view.compact.toggle" checked={look?.compact === true} />
        <Item kind="view.reduced_motion.toggle" checked={look?.reducedMotion === true} />
        <Separator />
        <Item kind="view.zoom.in" />
        <Item kind="view.zoom.out" />
        <Item kind="view.zoom.reset" />
        {isMainWindow && (
          <>
            <Separator />
            {panels.map((panel) => (
              <Item
                key={panel.id}
                kind={panel.command}
                checked={isPanelVisible(layout, panel.id)}
              />
            ))}
            <Item kind="view.docks.toggle" />
            <Sub label="Layout">
              <Menubar.Label className={menuLabel}>Built in</Menubar.Label>
              {layoutPresets.map((preset) => (
                <Menubar.Item
                  key={preset.id}
                  className={menuItem}
                  onSelect={() => {
                    applyLayout(preset.id);
                  }}
                >
                  <span className="flex w-3.5 justify-center">
                    {settings?.layout.active === preset.id && <Check className="size-3.5" />}
                  </span>
                  {preset.name}
                </Menubar.Item>
              ))}
              {settings && settings.layout.saved.length > 0 && (
                <>
                  <Menubar.Label className={menuLabel}>Saved</Menubar.Label>
                  {settings.layout.saved.map((saved) => (
                    <Menubar.Item
                      key={saved.name}
                      className={menuItem}
                      onSelect={() => {
                        applyLayout(saved.name);
                      }}
                    >
                      <span className="flex w-3.5 justify-center">
                        {settings.layout.active === saved.name && <Check className="size-3.5" />}
                      </span>
                      {saved.name}
                    </Menubar.Item>
                  ))}
                </>
              )}
              <Separator />
              <Item kind="view.layout.save" />
              <Item kind="view.layout.reset" />
            </Sub>
          </>
        )}
      </Menu>

      <Menu label="Window">
        <Item kind="window.next_tab" />
        <Item kind="window.previous_tab" />
        <Separator />
        <Item kind="window.tear_off" disabled={!doc} />
        {!isMainWindow && <Item kind="window.move_to_main" disabled={!doc} />}
        <Separator />
        {isMainWindow && (
          <>
            <Item kind="app.home" />
            {workspaces.map((ws) => (
              <Item key={ws.id} kind={ws.command} />
            ))}
            <Separator />
          </>
        )}
        <Item kind="app.notifications" />
      </Menu>
    </Menubar.Root>
  );
}
