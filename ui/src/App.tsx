import { useEffect, useState } from "react";
import { TooltipProvider } from "./components/Tooltip";
import { type AppInfo, inDesktopApp, ipc, onCoreEvent } from "./core/ipc";
import { JobsPanel } from "./shell/JobsPanel";
import { useJobs } from "./stores/jobs";
import { applyAppearance, useSettings } from "./stores/settings";

export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const desktop = inDesktopApp();

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    // subscribe before the first fetch so no event between the two is missed
    const unlisten = onCoreEvent((event) => {
      if (event.topic === "job") useJobs.getState().apply(event.payload);
      if (event.topic === "settingsChanged") {
        useSettings.getState().set(event.payload);
        applyAppearance(event.payload);
      }
    });

    Promise.all([ipc.appInfo(), ipc.getSettings(), ipc.listJobs()])
      .then(([appInfo, settings, jobs]) => {
        if (disposed) return;
        setInfo(appInfo);
        useSettings.getState().set(settings);
        applyAppearance(settings);
        useJobs.getState().replaceAll(jobs);
      })
      .catch((e: unknown) => setError(String(e)));

    return () => {
      disposed = true;
      unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [desktop]);

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full flex-col">
        <header className="flex h-11 shrink-0 items-center gap-3 border-b border-line px-4">
          <span className="text-sm font-semibold tracking-wide">Vellum</span>
          {info && <span className="text-xs text-muted">v{info.version}</span>}
        </header>
        <div className="flex min-h-0 flex-1">
          <main className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
            <h1 className="text-2xl font-semibold">Drop anything here</h1>
            <p className="max-w-md text-sm text-muted">
              The editors and quick tools arrive in the next phases. For now this window proves the
              core is wired up: try the sample job on the right.
            </p>
            {!desktop && (
              <p className="max-w-md rounded-panel border border-line bg-surface px-4 py-3 text-sm text-muted">
                You are viewing the UI in a plain browser. Run <code>pnpm tauri dev</code> from the
                repo root to start the desktop app.
              </p>
            )}
            {info?.settingsNotice && (
              <p className="max-w-md rounded-panel border border-line bg-surface px-4 py-3 text-sm">
                {info.settingsNotice}
              </p>
            )}
            {error && (
              <p className="max-w-md text-sm text-danger">
                Could not talk to the app core: {error}. Restart Vellum, and if it keeps happening
                report it with this message.
              </p>
            )}
          </main>
          <JobsPanel canRun={desktop && !error} />
        </div>
      </div>
    </TooltipProvider>
  );
}
