import { create } from "zustand";

import type { JobId } from "../bindings/JobId";
import type { JobSnapshot } from "../bindings/JobSnapshot";

export type NotificationTone = "info" | "good" | "bad";

export interface AppNotification {
  id: number;
  title: string;
  body: string | null;
  tone: NotificationTone;
  createdMs: number;
  read: boolean;
  /** a file the notification is about, opened when it is clicked */
  path: string | null;
  job: JobId | null;
}

const KEEP = 100;

interface NotificationsState {
  /** newest first */
  items: AppNotification[];
  /** ids still showing as a toast */
  toasts: number[];
  push: (notification: Omit<AppNotification, "id" | "createdMs" | "read">) => number;
  dismissToast: (id: number) => void;
  markAllRead: () => void;
  remove: (id: number) => void;
  clear: () => void;
}

let nextId = 1;

export const useNotifications = create<NotificationsState>()((set) => ({
  items: [],
  toasts: [],
  push: (notification) => {
    const id = nextId++;
    set((state) => ({
      items: [{ ...notification, id, createdMs: Date.now(), read: false }, ...state.items].slice(
        0,
        KEEP,
      ),
      // only the newest few as toasts, the rest wait in the centre
      toasts: [id, ...state.toasts].slice(0, 3),
    }));
    return id;
  },
  dismissToast: (id) => {
    set((state) => ({ toasts: state.toasts.filter((toast) => toast !== id) }));
  },
  markAllRead: () => {
    set((state) => ({ items: state.items.map((item) => ({ ...item, read: true })) }));
  },
  remove: (id) => {
    set((state) => ({
      items: state.items.filter((item) => item.id !== id),
      toasts: state.toasts.filter((toast) => toast !== id),
    }));
  },
  clear: () => {
    set({ items: [], toasts: [] });
  },
}));

export function unreadCount(items: AppNotification[]): number {
  return items.filter((item) => !item.read).length;
}

/** the file a finished job made, if it says so in its output */
export function outputPath(job: JobSnapshot): string | null {
  const output = job.output;
  if (typeof output !== "object" || output === null || Array.isArray(output)) return null;
  const path = output.path;
  return typeof path === "string" ? path : null;
}

/** the notification a job earns when it finishes, or null if it earns none */
export function notificationForJob(
  job: JobSnapshot,
): Omit<AppNotification, "id" | "createdMs" | "read"> | null {
  if (job.background) return null;
  const base = { job: job.id, path: outputPath(job) };
  switch (job.state) {
    case "succeeded":
      return {
        ...base,
        tone: "good",
        title: `${job.name} is done`,
        body: base.path ? "Click to open the result." : null,
      };
    case "failed":
      return { ...base, tone: "bad", title: `${job.name} failed`, body: job.error };
    default:
      return null;
  }
}
