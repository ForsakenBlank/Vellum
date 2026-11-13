import { clsx } from "clsx";
import { Pause, Play, RotateCcw, Trash2, X } from "lucide-react";
import { useMemo } from "react";

import type { JobSnapshot } from "../bindings/JobSnapshot";
import { Button, IconButton } from "../components/Button";
import { ProgressBar } from "../components/ProgressBar";
import { perform } from "../stores/app";
import { isFinished, orderJobs, useJobs } from "../stores/jobs";
import { describeJob } from "./jobText";

export function JobsPanel() {
  const jobs = useJobs((state) => state.jobs);
  const pending = useJobs((state) => state.pending);
  // housekeeping jobs stay out of the panel
  const visible = useMemo(
    () => orderJobs(jobs, pending).filter((job) => !job.background),
    [jobs, pending],
  );
  const anyFinished = visible.some(isFinished);

  return (
    <aside
      aria-label="Jobs"
      className="flex w-80 shrink-0 flex-col border-l border-line bg-surface"
    >
      <header className="flex h-11 shrink-0 items-center justify-between border-b border-line px-3">
        <h2 className="text-sm font-semibold">Jobs</h2>
        {anyFinished && (
          <Button
            tip="Remove finished jobs from this list"
            className="h-7 px-2 text-xs"
            onClick={() => void perform((api) => api.jobs.clearFinished())}
          >
            Clear finished
          </Button>
        )}
      </header>

      {visible.length === 0 ? (
        <p className="px-4 py-6 text-sm text-ink-soft">
          Nothing is running. Exports, conversions and other slow work will show up here.
        </p>
      ) : (
        <ul className="flex-1 overflow-y-auto">
          {visible.map((job) => (
            <JobRow key={job.id} job={job} />
          ))}
        </ul>
      )}
    </aside>
  );
}

function JobRow({ job }: { job: JobSnapshot }) {
  const tone =
    job.state === "succeeded"
      ? "good"
      : job.state === "failed"
        ? "bad"
        : job.state === "running"
          ? "accent"
          : "idle";
  const waiting = job.state === "queued" && job.progress === null;

  return (
    <li className="border-b border-line px-3 py-2.5">
      <div className="flex items-center gap-1">
        <span className="min-w-0 flex-1 truncate text-sm" title={job.name}>
          {job.name}
        </span>
        <JobActions job={job} />
      </div>
      <p
        className={clsx(
          "mt-0.5 mb-2 text-xs select-text",
          job.state === "failed" ? "text-bad" : "text-ink-soft",
        )}
      >
        {describeJob(job)}
      </p>
      {!waiting && job.state !== "cancelled" && (
        <ProgressBar value={job.progress} label={`${job.name} progress`} tone={tone} />
      )}
    </li>
  );
}

function JobActions({ job }: { job: JobSnapshot }) {
  const { id } = job;
  const pause = (
    <IconButton label="Pause" onClick={() => void perform((api) => api.jobs.pause(id))}>
      <Pause className="size-4" />
    </IconButton>
  );
  const resume = (
    <IconButton label="Resume" onClick={() => void perform((api) => api.jobs.resume(id))}>
      <Play className="size-4" />
    </IconButton>
  );
  const cancel = (
    <IconButton label="Cancel" onClick={() => void perform((api) => api.jobs.cancel(id))}>
      <X className="size-4" />
    </IconButton>
  );
  const retry = job.canRetry && (
    <IconButton label="Try again" onClick={() => void perform((api) => api.jobs.retry(id))}>
      <RotateCcw className="size-4" />
    </IconButton>
  );
  const remove = (
    <IconButton label="Remove from list" onClick={() => void perform((api) => api.jobs.remove(id))}>
      <Trash2 className="size-4" />
    </IconButton>
  );

  switch (job.state) {
    case "queued":
    case "running":
      return (
        <>
          {pause}
          {cancel}
        </>
      );
    case "paused":
      return (
        <>
          {resume}
          {cancel}
        </>
      );
    case "pausing":
      return cancel;
    case "cancelling":
      return null;
    case "succeeded":
      return remove;
    case "failed":
    case "cancelled":
      return (
        <>
          {retry}
          {remove}
        </>
      );
  }
}
