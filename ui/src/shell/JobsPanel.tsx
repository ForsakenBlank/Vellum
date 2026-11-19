import type { JobInfo } from "../bindings/JobInfo";
import { IconButton } from "../components/IconButton";
import { CloseIcon, PauseIcon, PlayIcon, RetryIcon } from "../components/icons";
import { formatEta, formatPercent } from "../core/format";
import { ipc } from "../core/ipc";
import { activeJobCount, useJobs } from "../stores/jobs";

// errors from job actions are rare races (the job finished as you clicked), so log and move on
function run(action: Promise<unknown>) {
  action.catch((e: unknown) => console.warn("job action failed:", e));
}

const statusText: Record<JobInfo["status"], string> = {
  queued: "Waiting",
  running: "Running",
  paused: "Paused",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

function JobRow({ job }: { job: JobInfo }) {
  const eta = job.status === "running" ? formatEta(job.etaSecs) : null;
  const finished =
    job.status === "completed" || job.status === "failed" || job.status === "cancelled";
  const detail = job.error ?? job.message ?? (eta ? `${eta} left` : statusText[job.status]);

  return (
    <li className="group rounded-panel border border-line bg-surface p-3">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{job.label}</span>
        {job.status === "running" && (
          <IconButton label="Pause" onClick={() => run(ipc.pauseJob(job.id))}>
            <PauseIcon />
          </IconButton>
        )}
        {job.status === "paused" && (
          <IconButton label="Resume" onClick={() => run(ipc.resumeJob(job.id))}>
            <PlayIcon />
          </IconButton>
        )}
        {(job.status === "failed" || job.status === "cancelled") && (
          <IconButton label="Try again" onClick={() => run(ipc.retryJob(job.id))}>
            <RetryIcon />
          </IconButton>
        )}
        <IconButton
          label={finished ? "Remove from list" : "Cancel"}
          onClick={() => run(finished ? ipc.removeJob(job.id) : ipc.cancelJob(job.id))}
        >
          <CloseIcon />
        </IconButton>
      </div>
      <div
        className="mt-2 h-1.5 overflow-hidden rounded-full bg-raised"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(job.progress * 100)}
      >
        <div
          className={`h-full rounded-full transition-[width] duration-(--motion-fast) ${
            job.status === "failed"
              ? "bg-danger"
              : job.status === "completed"
                ? "bg-success"
                : "bg-accent"
          }`}
          style={{ width: formatPercent(job.progress) }}
        />
      </div>
      <div className="mt-1.5 flex justify-between text-xs text-muted">
        <span className={`truncate ${job.status === "failed" ? "text-danger" : ""}`}>{detail}</span>
        <span>{formatPercent(job.progress)}</span>
      </div>
    </li>
  );
}

export function JobsPanel({ canRun }: { canRun: boolean }) {
  const jobs = useJobs();
  const active = activeJobCount(jobs);
  const list = jobs.order.map((id) => jobs.byId[id]).filter((j): j is JobInfo => !!j);

  return (
    <aside className="flex w-80 shrink-0 flex-col border-l border-line bg-bg">
      <header className="flex items-center justify-between px-4 py-3">
        <h2 className="text-sm font-semibold">
          Jobs {active > 0 && <span className="ml-1 text-muted">{active} active</span>}
        </h2>
        <button
          type="button"
          className="text-xs text-muted hover:text-fg disabled:opacity-40"
          disabled={list.length === active}
          onClick={() => run(ipc.clearFinishedJobs())}
        >
          Clear finished
        </button>
      </header>
      <ul className="flex flex-1 flex-col gap-2 overflow-y-auto px-4 pb-4">
        {list.length === 0 && (
          <li className="mt-8 text-center text-sm text-muted">
            Nothing running. Exports, conversions and AI tools show up here.
          </li>
        )}
        {list.map((job) => (
          <JobRow key={job.id} job={job} />
        ))}
      </ul>
      <footer className="border-t border-line p-4">
        <button
          type="button"
          disabled={!canRun}
          onClick={() => run(ipc.startSampleJob(5))}
          className="w-full rounded-md bg-accent px-3 py-2 text-sm font-medium text-white transition-opacity duration-(--motion-fast) hover:opacity-90 disabled:opacity-40"
        >
          Run a sample job
        </button>
      </footer>
    </aside>
  );
}
