import { useMemo, useState } from "react";

import type { JobResource } from "../bindings/JobResource";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { JobState } from "../bindings/JobState";
import { Button } from "../components/Button";
import { ProgressBar } from "../components/ProgressBar";
import { formatDuration, formatPercent } from "../lib/format";
import { inDesktopApp, ipc } from "../lib/ipc";
import { orderedJobs, useJobs } from "../state/jobs";

const stateLabels: Record<JobState, string> = {
  queued: "Waiting",
  running: "Running",
  pausing: "Pausing",
  paused: "Paused",
  cancelling: "Cancelling",
  waitingToRetry: "Retrying soon",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

const resourceLabels: Record<JobResource, string> = {
  cpu: "CPU",
  gpu: "GPU",
  io: "Disk and network",
};

export function JobsPanel() {
  const jobs = useJobs((s) => s.jobs);
  const waiting = useJobs((s) => s.waiting);
  const list = useMemo(
    () => orderedJobs({ jobs, waiting, orderRevision: 0, removedAt: {} }),
    [jobs, waiting],
  );
  const anyFinished = list.some((job) => isFinished(job.state));
  const [error, setError] = useState<string | null>(null);

  const run = (action: () => Promise<unknown>) => {
    setError(null);
    action().catch((err: unknown) => {
      setError(String(err));
    });
  };

  return (
    <aside aria-label="Jobs" className="flex h-full w-80 flex-col border-l border-line bg-panel">
      <header className="flex items-center justify-between border-b border-line px-3 py-2">
        <h2 className="text-sm font-semibold">Jobs</h2>
        <Button
          variant="ghost"
          tooltip="Remove finished jobs from this list"
          disabled={!anyFinished}
          onClick={() => {
            run(ipc.clearFinishedJobs);
          }}
        >
          Clear finished
        </Button>
      </header>

      {error && (
        <p role="alert" className="border-b border-line px-3 py-2 text-xs text-danger">
          {error}
        </p>
      )}

      <ol className="flex-1 overflow-y-auto">
        {list.length === 0 ? (
          <li className="px-3 py-6 text-center text-sm text-muted">
            No jobs yet. Exports, conversions and AI tools show up here while they run.
          </li>
        ) : (
          list.map((job) => <JobRow key={job.id} job={job} run={run} />)
        )}
      </ol>

      {import.meta.env.DEV && inDesktopApp && <SampleJobs run={run} />}
    </aside>
  );
}

/** the moving "working on it" bar only makes sense while a job actually runs */
function progressToShow(job: JobSnapshot): number | null {
  if (job.state === "completed") return 1;
  if (job.progress !== null) return job.progress;
  return job.state === "running" ? null : 0;
}

function isFinished(state: JobState) {
  return state === "completed" || state === "failed" || state === "cancelled";
}

interface JobRowProps {
  job: JobSnapshot;
  run: (action: () => Promise<unknown>) => void;
}

function JobRow({ job, run }: JobRowProps) {
  const { id, state } = job;
  const canPause = state === "queued" || state === "running" || state === "waitingToRetry";
  const canResume = state === "paused" || state === "pausing";
  const canCancel = !isFinished(state) && state !== "cancelling";
  const canRetry = state === "failed" || state === "cancelled";

  const details = [stateLabels[state], resourceLabels[job.resource]];
  if (job.progress !== null && !isFinished(state)) details.push(formatPercent(job.progress));
  if (job.etaSecs !== null) details.push(`${formatDuration(job.etaSecs)} left`);
  if (state === "completed") details.push(`took ${formatDuration(job.elapsedSecs)}`);
  if (job.attempt > 0 && !isFinished(state)) details.push(`try ${job.attempt + 1}`);

  const tone =
    state === "failed"
      ? "danger"
      : state === "completed"
        ? "success"
        : state === "paused" || state === "cancelled"
          ? "muted"
          : "accent";

  return (
    <li className="flex flex-col gap-1.5 border-b border-line px-3 py-2.5">
      <div className="truncate text-sm" title={job.title}>
        {job.title}
      </div>
      <ProgressBar value={progressToShow(job)} label={`${job.title} progress`} tone={tone} />
      <div className="text-xs text-muted">{details.join(" · ")}</div>
      {job.message && !isFinished(state) && <div className="text-xs text-muted">{job.message}</div>}
      {state === "failed" && job.error && (
        <div className="text-xs text-danger">{job.error}. Press Retry to run it again.</div>
      )}
      <div className="flex gap-1.5">
        {canPause && (
          <Button
            tooltip="Pause this job and free its slot for others"
            onClick={() => {
              run(() => ipc.pauseJob(id));
            }}
          >
            Pause
          </Button>
        )}
        {canResume && (
          <Button
            tooltip="Carry on from where it paused"
            onClick={() => {
              run(() => ipc.resumeJob(id));
            }}
          >
            Resume
          </Button>
        )}
        {canCancel && (
          <Button
            variant="danger"
            tooltip="Stop this job. Your original files are not touched."
            onClick={() => {
              run(() => ipc.cancelJob(id));
            }}
          >
            Cancel
          </Button>
        )}
        {canRetry && (
          <Button
            tooltip="Run this job again from the start"
            onClick={() => {
              run(() => ipc.retryJob(id));
            }}
          >
            Retry
          </Button>
        )}
        {isFinished(state) && (
          <Button
            variant="ghost"
            tooltip="Remove from this list"
            onClick={() => {
              run(() => ipc.removeJob(id));
            }}
          >
            Remove
          </Button>
        )}
      </div>
    </li>
  );
}

function SampleJobs({ run }: { run: JobRowProps["run"] }) {
  return (
    <footer className="flex flex-col gap-1.5 border-t border-line p-3">
      <h3 className="text-xs font-semibold text-muted">Sample jobs (development only)</h3>
      <div className="flex flex-wrap gap-1.5">
        <Button
          tooltip="Start a 10 second job on a CPU slot"
          onClick={() => {
            run(() => ipc.startSampleJob(10, "cpu", false));
          }}
        >
          CPU job
        </Button>
        <Button
          tooltip="Start a 10 second job on a GPU slot"
          onClick={() => {
            run(() => ipc.startSampleJob(10, "gpu", false));
          }}
        >
          GPU job
        </Button>
        <Button
          tooltip="Start a job that fails halfway once, then retries by itself"
          onClick={() => {
            run(() => ipc.startSampleJob(6, "cpu", true));
          }}
        >
          Flaky job
        </Button>
      </div>
    </footer>
  );
}
