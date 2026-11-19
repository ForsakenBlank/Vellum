import { X } from "lucide-react";
import { useEffect, useRef } from "react";

import { IconButton } from "../components/Button";
import { useUi, type Toast } from "../state/ui";
import { levelColours, levelIcons } from "./notify";

const LIFETIME_MS = 5000;

/** short lived messages in the bottom right. hovering one keeps it on screen. */
export function Toasts() {
  const toasts = useUi((s) => s.toasts);
  return (
    <div
      role="region"
      aria-label="Messages"
      className="pointer-events-none fixed right-3 bottom-[calc(var(--statusbar)+0.75rem)] z-[var(--z-toast)] flex w-80 flex-col gap-2"
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} />
      ))}
    </div>
  );
}

function ToastCard({ toast }: { toast: Toast }) {
  const dismiss = useUi((s) => s.dismissToast);
  const timer = useRef<number | null>(null);
  const Icon = levelIcons[toast.level];

  const start = () => {
    // errors stay until dismissed, there may be something to act on
    if (toast.level === "error") return;
    timer.current = window.setTimeout(() => {
      dismiss(toast.id);
    }, LIFETIME_MS);
  };
  const stop = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
  };

  useEffect(() => {
    start();
    return stop;
    // the timer belongs to this toast only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div
      role={toast.level === "error" ? "alert" : "status"}
      onPointerEnter={stop}
      onPointerLeave={start}
      className="pointer-events-auto flex animate-slide-up gap-2.5 rounded-lg bg-elevated px-3 py-2.5 shadow-pop"
    >
      <Icon className={`mt-0.5 size-4 shrink-0 ${levelColours[toast.level]}`} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <p className="text-sm font-medium">{toast.title}</p>
        {toast.body && <p className="text-xs text-fg-muted">{toast.body}</p>}
        {toast.action && (
          <button
            type="button"
            className="mt-1 self-start text-xs font-medium text-accent-text hover:underline"
            onClick={() => {
              toast.action?.run();
              dismiss(toast.id);
            }}
          >
            {toast.action.label}
          </button>
        )}
      </div>
      <IconButton
        size="sm"
        label="Dismiss"
        icon={<X className="size-3" />}
        onClick={() => {
          dismiss(toast.id);
        }}
      />
    </div>
  );
}
