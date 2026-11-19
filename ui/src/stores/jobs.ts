import { create } from "zustand";
import type { JobEvent } from "../bindings/JobEvent";
import type { JobInfo } from "../bindings/JobInfo";

export interface JobsSnapshot {
  byId: Record<number, JobInfo>;
  // ids in the order the queue reported them, oldest first
  order: number[];
}

export const emptyJobs: JobsSnapshot = { byId: {}, order: [] };

export function jobsFromList(list: JobInfo[]): JobsSnapshot {
  const byId: Record<number, JobInfo> = {};
  for (const job of list) byId[job.id] = job;
  return { byId, order: list.map((j) => j.id) };
}

// pure so it can be tested without tauri
export function reduceJobs(state: JobsSnapshot, event: JobEvent): JobsSnapshot {
  switch (event.type) {
    case "changed": {
      const { job } = event;
      const known = job.id in state.byId;
      return {
        byId: { ...state.byId, [job.id]: job },
        order: known ? state.order : [...state.order, job.id],
      };
    }
    case "progress": {
      const job = state.byId[event.id];
      // progress for a job we never saw added, the next full refresh will pick it up
      if (!job) return state;
      return {
        ...state,
        byId: {
          ...state.byId,
          [event.id]: {
            ...job,
            progress: event.progress,
            etaSecs: event.etaSecs,
            message: event.message,
          },
        },
      };
    }
    case "removed": {
      if (!(event.id in state.byId)) return state;
      const byId = { ...state.byId };
      delete byId[event.id];
      return { byId, order: state.order.filter((id) => id !== event.id) };
    }
  }
}

export function activeJobCount(state: JobsSnapshot): number {
  return state.order.filter((id) => {
    const status = state.byId[id]?.status;
    return status === "queued" || status === "running" || status === "paused";
  }).length;
}

interface JobsStore extends JobsSnapshot {
  replaceAll: (list: JobInfo[]) => void;
  apply: (event: JobEvent) => void;
}

export const useJobs = create<JobsStore>()((set) => ({
  ...emptyJobs,
  replaceAll: (list) => set(jobsFromList(list)),
  apply: (event) => set((state) => reduceJobs(state, event)),
}));
