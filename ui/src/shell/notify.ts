import { CircleCheck, CircleExclamationPoint, CircleX, Info, type LucideIcon } from "lucide-react";

import type { NotificationAction } from "../bindings/NotificationAction";
import type { NotificationLevel } from "../bindings/NotificationLevel";
import { ipc } from "../lib/ipc";
import { useUi, type ToastLevel } from "../state/ui";
import { revealPanel } from "./actions";

export const levelIcons: Record<NotificationLevel | ToastLevel, LucideIcon> = {
  info: Info,
  success: CircleCheck,
  warning: CircleExclamationPoint,
  error: CircleX,
};

export const levelColours: Record<NotificationLevel | ToastLevel, string> = {
  info: "text-info",
  success: "text-success",
  warning: "text-warning",
  error: "text-danger",
};

/** does what a notification's button says */
export function performAction(action: NotificationAction) {
  const fail = (err: unknown) => {
    useUi.getState().toast({ level: "error", title: "That did not work", body: String(err) });
  };
  switch (action.type) {
    case "openPath":
      ipc.openResult(action.path, false).catch(fail);
      break;
    case "showJob":
      revealPanel("jobs");
      break;
    case "retryJob":
      ipc.retryJob(action.job).catch(fail);
      revealPanel("jobs");
      break;
  }
}
