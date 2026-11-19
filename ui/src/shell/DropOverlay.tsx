import { Ban, FileInput } from "lucide-react";

import { useUi } from "../state/ui";
import { quickTool } from "../workspaces/quickTools";
import { workspaceInfo } from "../workspaces/workspaces";

/**
 * covers the window while files are dragged over it and says what dropping will do. over a
 * quick tool card the overlay steps back so the card itself can light up.
 */
export function DropOverlay() {
  const plan = useUi((s) => s.drop);
  const tool = useUi((s) => s.dropTool);
  if (!plan) return null;

  const onCard = tool !== null;
  const Icon =
    plan.action === "reject"
      ? Ban
      : plan.action === "tool"
        ? (quickTool(plan.tool)?.icon ?? FileInput)
        : workspaceInfo(plan.workspace).icon;

  return (
    <div
      aria-live="polite"
      data-testid="drop-overlay"
      className={`pointer-events-none fixed inset-0 z-[var(--z-drop)] flex items-end justify-center p-6 transition-colors duration-(--motion-base) ${
        onCard ? "bg-transparent" : "items-center bg-scrim backdrop-blur-[1px]"
      }`}
    >
      {!onCard && (
        <div
          aria-hidden
          className="absolute inset-3 rounded-2xl border-2 border-dashed border-accent-line"
        />
      )}
      <div
        role="status"
        className={`flex animate-pop-in items-center gap-3 rounded-xl px-4 py-3 shadow-pop ${
          plan.action === "reject" ? "bg-elevated" : "bg-panel"
        }`}
      >
        <span
          className={`flex size-10 items-center justify-center rounded-lg ${
            plan.action === "reject" ? "bg-danger/15 text-danger" : "bg-accent text-accent-fg"
          }`}
        >
          <Icon className="size-5" strokeWidth={1.8} />
        </span>
        <div className="flex flex-col">
          <span className="text-base font-semibold tracking-tight">{plan.title}</span>
          {plan.detail && <span className="text-xs text-fg-muted">{plan.detail}</span>}
        </div>
      </div>
    </div>
  );
}
