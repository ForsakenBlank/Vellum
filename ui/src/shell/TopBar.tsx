import { clsx } from "clsx";
import { LoaderCircle, Search, Settings } from "lucide-react";

import { useShortcut } from "../commands/registry";
import { IconButton } from "../components/Button";
import { Kbd } from "../components/Kbd";
import { Tip } from "../components/Tip";
import { togglePanelIn } from "../dock/state";
import { useJobs } from "../stores/jobs";
import { useUi } from "../stores/ui";
import type { Workspace } from "../workspaces/types";
import { LayoutMenu } from "./LayoutMenu";
import { NotificationCentre } from "./Notifications";
import { Tabs } from "./Tabs";

/** the strip across the top: open files, then search, jobs, notifications and settings */
export function TopBar({ workspace }: { workspace: Workspace }) {
  const paletteKey = useShortcut("app.palette");
  const settingsKey = useShortcut("app.settings");

  return (
    <header className="flex h-10 shrink-0 items-stretch border-b border-line bg-canvas">
      <Tabs />
      <div className="flex shrink-0 items-center gap-1 border-l border-line px-2">
        <Tip label="Find any command, setting or file" shortcut={paletteKey}>
          <button
            type="button"
            aria-label="Search"
            onClick={() => {
              useUi.getState().setPaletteOpen(true);
            }}
            className="v-focus mr-1 flex h-7 w-44 items-center gap-2 rounded-md border border-line bg-surface px-2 text-xs text-ink-faint transition-colors duration-(--v-fast) hover:border-line-strong hover:text-ink-soft"
          >
            <Search className="size-3.5" aria-hidden />
            <span className="flex-1 text-left">Search</span>
            {paletteKey && <Kbd shortcut={paletteKey} className="h-4" />}
          </button>
        </Tip>
        <JobsIndicator workspace={workspace} />
        <NotificationCentre />
        <LayoutMenu workspace={workspace} />
        <IconButton
          label="Settings"
          shortcut={settingsKey}
          onClick={() => {
            useUi.getState().openSettings();
          }}
        >
          <Settings className="size-4" />
        </IconButton>
      </div>
    </header>
  );
}

/** a spinner with a count while jobs are running. it shows the jobs panel */
function JobsIndicator({ workspace }: { workspace: Workspace }) {
  const active = useJobs(
    (state) =>
      Object.values(state.jobs).filter(
        (job) => !job.background && ["running", "queued", "pausing"].includes(job.state),
      ).length,
  );
  const shortcut = useShortcut("view.panel.jobs");
  if (active === 0) return null;
  return (
    <IconButton
      label={active === 1 ? "1 job running. Show jobs" : `${active} jobs running. Show jobs`}
      shortcut={shortcut}
      className={clsx("w-auto gap-1 px-1.5 text-xs text-accent")}
      onClick={() => void togglePanelIn(workspace, "jobs")}
    >
      <LoaderCircle className="size-3.5 animate-spin" />
      {active}
    </IconButton>
  );
}
