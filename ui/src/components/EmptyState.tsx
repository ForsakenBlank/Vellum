import { clsx } from "clsx";
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  body?: ReactNode;
  /** buttons or links under the text */
  children?: ReactNode;
  /** smaller, for panels */
  compact?: boolean;
}

/** what a place says when there is nothing in it yet, and what to do about that */
export function EmptyState({ icon: Icon, title, body, children, compact }: EmptyStateProps) {
  return (
    <div
      className={clsx(
        "flex h-full flex-col items-center justify-center text-center",
        compact ? "gap-1.5 px-4 py-6" : "gap-3 px-8 py-12",
      )}
    >
      <div
        className={clsx(
          "flex items-center justify-center rounded-xl border border-line bg-raised text-ink-soft",
          compact ? "mb-1 size-9" : "mb-1 size-12",
        )}
      >
        <Icon className={compact ? "size-4" : "size-5"} strokeWidth={1.75} aria-hidden />
      </div>
      <p className={clsx("font-medium text-ink", compact ? "text-sm" : "text-base")}>{title}</p>
      {body && (
        <p className={clsx("max-w-xs text-ink-soft", compact ? "text-xs" : "text-sm")}>{body}</p>
      )}
      {children && <div className="mt-2 flex flex-wrap justify-center gap-2">{children}</div>}
    </div>
  );
}
