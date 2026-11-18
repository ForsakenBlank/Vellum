import { ToggleGroup } from "radix-ui";

import type { Theme } from "../bindings/Theme";
import { Tooltip } from "../components/Tooltip";
import { inDesktopApp } from "../lib/ipc";
import { useSettings } from "../state/settings";
import { workspaces } from "../workspaces/workspaces";
import { JobsPanel } from "./JobsPanel";

const themes: { value: Theme; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "high-contrast", label: "High contrast" },
];

export function AppShell() {
  const settings = useSettings((s) => s.settings);
  const settingsError = useSettings((s) => s.error);
  const update = useSettings((s) => s.update);
  const theme = settings?.appearance.theme ?? "dark";

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between border-b border-line bg-panel px-4 py-2">
        <div className="flex items-baseline gap-2">
          <span className="text-base font-semibold tracking-tight">Vellum</span>
          <span className="text-xs text-muted">v{__APP_VERSION__}</span>
        </div>
        <ToggleGroup.Root
          type="single"
          aria-label="Theme"
          value={theme}
          disabled={!settings}
          onValueChange={(value) => {
            if (value) void update({ appearance: { theme: value as Theme } });
          }}
          className="flex rounded-md border border-line p-0.5"
        >
          {themes.map((t) => (
            <Tooltip key={t.value} text={`Use the ${t.label.toLowerCase()} theme`}>
              <ToggleGroup.Item
                value={t.value}
                className="rounded px-2 py-0.5 text-xs text-muted data-[state=on]:bg-accent data-[state=on]:text-white"
              >
                {t.label}
              </ToggleGroup.Item>
            </Tooltip>
          ))}
        </ToggleGroup.Root>
      </header>

      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Workspaces"
          className="flex w-44 flex-col gap-0.5 border-r border-line p-2"
        >
          {workspaces.map((ws) => (
            <Tooltip
              key={ws.id}
              text={`The ${ws.label.toLowerCase()} workspace arrives in phase ${ws.phase}`}
            >
              <span>
                <button
                  type="button"
                  disabled
                  className="w-full cursor-not-allowed rounded-md px-2 py-1.5 text-left text-sm text-muted"
                >
                  {ws.label}
                </button>
              </span>
            </Tooltip>
          ))}
        </nav>

        <main className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
          <h1 className="text-2xl font-semibold">Drop a file to get started</h1>
          <p className="max-w-md text-sm text-muted">
            Workspaces and quick tools arrive in the next phases. For now this window shows the job
            queue and settings working end to end.
          </p>
          {!inDesktopApp && (
            <p
              role="status"
              className="max-w-md rounded-md border border-line p-3 text-sm text-muted"
            >
              This page is running in a browser, so nothing reaches the Rust side. Start the desktop
              app with <code>pnpm tauri dev</code> to try the job queue.
            </p>
          )}
          {settingsError && (
            <p role="alert" className="max-w-md text-sm text-danger">
              Settings could not be saved: {settingsError}. Try again, or restart Vellum if it keeps
              happening.
            </p>
          )}
        </main>

        <JobsPanel />
      </div>
    </div>
  );
}
