import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
}

/** what a panel or page shows when it has nothing yet, with a hint of what to do next */
export function EmptyState({
  icon: Icon,
  title,
  children,
  actions,
  compact = false,
}: EmptyStateProps) {
  return (
    <div
      className={`flex flex-col items-center justify-center text-center ${compact ? "gap-2 px-4 py-6" : "gap-3 px-6 py-12"}`}
    >
      <div
        className={`flex items-center justify-center rounded-xl border border-line bg-raised text-fg-subtle ${compact ? "size-9" : "size-12"}`}
      >
        <Icon className={compact ? "size-4" : "size-5"} strokeWidth={1.6} />
      </div>
      <div className="flex max-w-80 flex-col gap-1">
        <p className={`font-medium text-fg ${compact ? "text-sm" : "text-base"}`}>{title}</p>
        {children && <div className="text-xs text-fg-muted">{children}</div>}
      </div>
      {actions && <div className="mt-1 flex gap-2">{actions}</div>}
    </div>
  );
}
