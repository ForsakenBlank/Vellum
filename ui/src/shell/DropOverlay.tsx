import { clsx } from "clsx";
import { Ban, FileUp } from "lucide-react";

import { describeDrop, targetFromKey, useDrop } from "./drop";

/** says what will happen while files from the desktop hover over the window */
export function DropOverlay() {
  const paths = useDrop((state) => state.paths);
  const plan = useDrop((state) => state.plan);
  const targetKey = useDrop((state) => state.targetKey);
  if (!paths) return null;

  const summary = plan ? describeDrop(plan, targetFromKey(targetKey)) : null;
  const ok = summary?.ok ?? true;
  // over a target the card itself lights up, so the message moves out of the way
  const onTarget = targetKey !== null;

  return (
    <div
      className={clsx(
        "v-fade pointer-events-none fixed inset-0 z-[60] flex justify-center p-6",
        onTarget ? "items-end bg-black/10" : "items-center bg-black/35",
      )}
    >
      {!onTarget && (
        <div className="absolute inset-3 rounded-2xl border-2 border-dashed border-accent/70" />
      )}
      <div
        role="status"
        aria-live="polite"
        className={clsx(
          "v-rise flex max-w-lg items-center gap-3.5 rounded-xl border bg-raised px-4 py-3 shadow-modal",
          ok ? "border-line-strong" : "border-bad/60",
        )}
      >
        <div
          className={clsx(
            "flex size-10 shrink-0 items-center justify-center rounded-lg",
            ok ? "bg-accent-soft text-accent" : "bg-bad/15 text-bad",
          )}
        >
          {ok ? <FileUp className="size-5" aria-hidden /> : <Ban className="size-5" aria-hidden />}
        </div>
        <div className="min-w-0">
          <p className="font-medium">{summary?.headline ?? "Drop to open"}</p>
          <p className="truncate text-sm text-ink-soft">
            {summary?.detail ?? (paths.length === 1 ? "1 file" : `${paths.length} files`)}
          </p>
        </div>
      </div>
    </div>
  );
}
