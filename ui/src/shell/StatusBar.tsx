import { DropdownMenu } from "radix-ui";
import { Check, CloudCheck, CloudAlert, LayoutTemplate, ListChecks } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { menuContent, menuItem, menuLabel, menuSeparator } from "../components/menuStyles";
import { Tooltip } from "../components/Tooltip";
import { layoutPresets } from "../lib/dock";
import { formatAgo } from "../lib/format";
import { isMainWindow } from "../lib/window";
import { runCommand } from "../state/commands";
import { activeDocument, useDocuments } from "../state/documents";
import { useJobs } from "../state/jobs";
import { useSettings } from "../state/settings";
import { useUi } from "../state/ui";
import { quickTool } from "../workspaces/quickTools";
import { workspaceInfo } from "../workspaces/workspaces";
import { applyLayout, revealPanel } from "./actions";

const cell =
  "flex h-full items-center gap-1.5 rounded-sm px-2 transition-colors duration-(--motion-fast) hover:bg-raised hover:text-fg";

export function StatusBar({ onSaveLayout }: { onSaveLayout: () => void }) {
  const doc = useDocuments(activeDocument);
  const empty = useDocuments((s) => s.emptyWorkspace);
  const autosave = useUi((s) => s.autosave);
  const jobs = useJobs((s) => s.jobs);
  const settings = useSettings((s) => s.settings);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Date.now());
    }, 15_000);
    return () => {
      window.clearInterval(timer);
    };
  }, []);

  const running = useMemo(
    () => Object.values(jobs).filter((j) => j.state === "running").length,
    [jobs],
  );
  const waiting = useMemo(
    () => Object.values(jobs).filter((j) => j.state === "queued").length,
    [jobs],
  );

  const where = doc
    ? `${doc.tool ? (quickTool(doc.tool)?.title ?? "Quick tool") : workspaceInfo(doc.workspace).label} · ${doc.title}`
    : empty
      ? workspaceInfo(empty).label
      : "Home";

  const layoutName =
    layoutPresets.find((p) => p.id === settings?.layout.active)?.name ??
    settings?.layout.active ??
    "Default";

  return (
    <footer className="flex h-(--statusbar) shrink-0 items-center gap-1 border-t border-line bg-panel px-1.5 text-2xs text-fg-subtle">
      <span className="truncate px-2">{where}</span>
      <span className="flex-1" />

      <Tooltip
        text={
          autosave.error ??
          (settings?.autosave.enabled === false
            ? "Autosave is off. Turn it on in settings to be safe from crashes."
            : "Open documents are saved in the background, so a crash loses nothing")
        }
      >
        <button
          type="button"
          className={cell}
          onClick={() => {
            useUi.getState().openSettings("autosave");
          }}
        >
          {autosave.error ? (
            <CloudAlert className="size-3 text-warning" />
          ) : (
            <CloudCheck className="size-3" />
          )}
          <span className="tabular">
            {settings?.autosave.enabled === false
              ? "Autosave off"
              : autosave.savedAtMs
                ? `Autosaved ${formatAgo(autosave.savedAtMs, now)}`
                : "Autosave on"}
          </span>
        </button>
      </Tooltip>

      <Tooltip text="Show the jobs panel" command="view.panel.jobs">
        <button
          type="button"
          className={cell}
          onClick={() => {
            if (isMainWindow) revealPanel("jobs");
          }}
        >
          <ListChecks className={`size-3 ${running > 0 ? "text-accent-text" : ""}`} />
          <span className="tabular">
            {running > 0
              ? `${running} running${waiting > 0 ? `, ${waiting} waiting` : ""}`
              : "No jobs running"}
          </span>
        </button>
      </Tooltip>

      {isMainWindow && (
        <DropdownMenu.Root>
          <Tooltip text="Switch or save panel layouts">
            <DropdownMenu.Trigger className={cell}>
              <LayoutTemplate className="size-3" />
              {layoutName}
            </DropdownMenu.Trigger>
          </Tooltip>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className={menuContent} side="top" align="end" sideOffset={6}>
              <DropdownMenu.Label className={menuLabel}>Built in</DropdownMenu.Label>
              {layoutPresets.map((preset) => (
                <DropdownMenu.Item
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
                </DropdownMenu.Item>
              ))}
              {settings && settings.layout.saved.length > 0 && (
                <>
                  <DropdownMenu.Label className={menuLabel}>Saved</DropdownMenu.Label>
                  {settings.layout.saved.map((saved) => (
                    <DropdownMenu.Item
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
                    </DropdownMenu.Item>
                  ))}
                </>
              )}
              <DropdownMenu.Separator className={menuSeparator} />
              <DropdownMenu.Item className={menuItem} onSelect={onSaveLayout}>
                <span className="w-3.5" />
                Save layout as…
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      )}

      <Tooltip text="Interface size. Click for 100%." command="view.zoom.reset">
        <button
          type="button"
          className={`${cell} tabular`}
          onClick={() => void runCommand("view.zoom.reset")}
        >
          {Math.round((settings?.appearance.uiScale ?? 1) * 100)}%
        </button>
      </Tooltip>
    </footer>
  );
}
