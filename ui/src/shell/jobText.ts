import type { JobSnapshot } from "../bindings/JobSnapshot";
import { formatDuration, formatEta, formatPercent } from "../lib/format";

/** the one line under a job's name that says where it stands */
export function describeJob(job: JobSnapshot): string {
  const percent = job.progress === null ? null : formatPercent(job.progress);
  switch (job.state) {
    case "queued":
      return job.attempt > 0
        ? `Waiting to try again (${job.attempt} of ${job.maxAttempts})`
        : "Waiting";
    case "running": {
      const parts = [percent, job.message, job.etaMs === null ? null : formatEta(job.etaMs)];
      return parts.filter(Boolean).join(" · ") || "Running";
    }
    case "pausing":
      return "Pausing";
    case "paused":
      return percent ? `Paused at ${percent}` : "Paused";
    case "cancelling":
      return "Cancelling";
    case "succeeded":
      return `Done in ${formatDuration(job.elapsedMs)}`;
    case "failed":
      return job.error ?? "Failed";
    case "cancelled":
      return "Cancelled";
  }
}
