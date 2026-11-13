import { Contrast, Moon, Sun } from "lucide-react";
import type { ReactNode } from "react";

import type { Theme } from "../bindings/Theme";
import { Button } from "../components/Button";
import { Segmented } from "../components/Segmented";
import { perform, useApp } from "../stores/app";
import { updateSettings, useSettings } from "../stores/settings";

const themes: { value: Theme; label: string; icon: ReactNode }[] = [
  { value: "dark", label: "Dark", icon: <Moon className="size-3.5" aria-hidden /> },
  { value: "light", label: "Light", icon: <Sun className="size-3.5" aria-hidden /> },
  {
    value: "highContrast",
    label: "High contrast",
    icon: <Contrast className="size-3.5" aria-hidden />,
  },
];

const statusText = {
  connecting: "Starting up",
  ready: "Everything is running",
  browser: "Open in a browser, without the desktop app around it",
  failed: "Could not reach the backend",
};

/**
 * the phase 0 home page: a check that the window, the backend, settings and
 * the job queue all talk to each other. phase 1 replaces it with the real one
 */
export function Home() {
  const status = useApp((state) => state.status);
  const info = useApp((state) => state.info);
  const appearance = useSettings((state) => state.sections?.appearance);
  const ready = status === "ready";

  return (
    <div className="mx-auto flex h-full max-w-2xl flex-col gap-4 overflow-y-auto px-8 py-10">
      <header className="mb-2">
        <h1 className="text-2xl font-semibold tracking-tight">Vellum</h1>
        <p className="mt-1 text-sm text-ink-soft">
          The foundations are in place. The tools arrive phase by phase.
        </p>
      </header>

      <Card title="App">
        <dl className="grid grid-cols-[8rem_1fr] gap-y-1.5 text-sm">
          <Row label="Status">{statusText[status]}</Row>
          {info && (
            <>
              <Row label="Version">
                {info.version}
                {info.debugBuild && " (debug build)"}
              </Row>
              <Row label="System">
                {info.os} {info.arch}
              </Row>
            </>
          )}
        </dl>
        {status === "browser" && (
          <p className="mt-3 text-sm text-ink-soft">
            Start the desktop app with <code className="text-ink select-text">pnpm tauri dev</code>{" "}
            to connect the backend.
          </p>
        )}
      </Card>

      <Card title="Theme" hint="Saved to your settings file and applied straight away.">
        {appearance ? (
          <Segmented
            label="Theme"
            value={appearance.theme}
            options={themes}
            onChange={(theme) => void updateSettings("appearance", { theme })}
          />
        ) : (
          <p className="text-sm text-ink-soft">Settings load once the backend is connected.</p>
        )}
      </Card>

      <Card
        title="Job queue"
        hint="Sample jobs stand in for exports until real ones exist. Try pausing, cancelling and retrying them in the panel on the right."
      >
        <div className="flex flex-wrap gap-2">
          <Button
            variant="primary"
            disabled={!ready}
            tip="Add a job that runs for 8 seconds"
            onClick={() => void perform((api) => api.jobs.runSample(8, false))}
          >
            Run a sample job
          </Button>
          <Button
            disabled={!ready}
            tip="Add a job that stops with an error half way"
            onClick={() => void perform((api) => api.jobs.runSample(4, true))}
          >
            Run one that fails
          </Button>
        </div>
      </Card>
    </div>
  );
}

function Card({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="rounded-lg border border-line bg-surface p-4">
      <h2 className="text-sm font-semibold">{title}</h2>
      {hint && <p className="mt-1 text-xs text-ink-soft">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-ink-soft">{label}</dt>
      <dd className="select-text">{children}</dd>
    </>
  );
}
