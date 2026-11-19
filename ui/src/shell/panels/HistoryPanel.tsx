import { Bookmark, Camera, GitBranch } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type { HistoryView } from "../../bindings/HistoryView";
import { IconButton } from "../../components/Button";
import { EmptyState } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { useDocuments } from "../../state/documents";
import { useUi } from "../../state/ui";
import { historyRows } from "./historyRows";

/** every step of the active document, with branches. click a step to go back to it. */
export function HistoryPanel() {
  const activeId = useDocuments((s) => s.activeId);
  const summary = useDocuments((s) => (s.activeId ? s.summaries[s.activeId] : undefined));
  const [view, setView] = useState<HistoryView | null>(null);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);

  useEffect(() => {
    if (!activeId || !inDesktopApp) return;
    let stale = false;
    ipc
      .documentHistory(activeId)
      .then((next) => {
        if (!stale) {
          setView(next);
          setLoadedFor(activeId);
        }
      })
      .catch(() => {
        if (!stale) setView(null);
      });
    return () => {
      stale = true;
    };
    // the summary changes with every step, which is when the tree needs reloading
  }, [activeId, summary]);

  const rows = useMemo(() => (view ? historyRows(view) : []), [view]);

  if (!activeId) {
    return (
      <EmptyState icon={GitBranch} title="No document open" compact>
        Every change you make shows up here, including ones on other branches after an undo.
      </EmptyState>
    );
  }
  if (!view || loadedFor !== activeId) {
    return (
      <div className="flex flex-col gap-2 p-3" aria-busy>
        <Skeleton className="h-5 w-2/3" />
        <Skeleton className="h-5 w-1/2" />
        <Skeleton className="h-5 w-3/5" />
      </div>
    );
  }

  const checkout = (node: number) => {
    ipc.checkoutHistory(activeId, node).catch((err: unknown) => {
      useUi.getState().toast({
        level: "error",
        title: "Could not go back to that step",
        body: `${String(err)}. The document stopped at the last step that worked.`,
      });
    });
  };

  const snapshot = () => {
    const name = `Snapshot ${view.snapshots.length + 1}`;
    void ipc
      .createSnapshot(activeId, name)
      .then(() => ipc.documentHistory(activeId))
      .then(setView);
  };

  return (
    <section aria-label="History" className="flex h-full min-h-0 flex-col">
      <div className="flex items-center justify-between px-3 py-1.5">
        <span className="text-xs text-fg-subtle">
          {rows.length - 1} {rows.length === 2 ? "step" : "steps"}
        </span>
        <IconButton
          size="sm"
          label="Take a snapshot of this step"
          icon={<Camera className="size-3.5" />}
          onClick={snapshot}
        />
      </div>
      <ol className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {rows.map(({ entry, depth, current }) => {
          const snap = view.snapshots.find((s) => s.node === entry.id);
          return (
            <li key={entry.id}>
              <button
                type="button"
                aria-current={current ? "step" : undefined}
                onClick={() => {
                  checkout(entry.id);
                }}
                style={{ paddingLeft: `${0.5 + depth * 0.9}rem` }}
                className={`flex h-(--row-sm) w-full items-center gap-2 rounded-md pr-2 text-left text-sm transition-colors duration-(--motion-fast) ${
                  current
                    ? "bg-accent-soft text-fg"
                    : entry.onCurrentPath
                      ? "text-fg-muted hover:bg-elevated hover:text-fg"
                      : "text-fg-subtle italic hover:bg-elevated"
                }`}
              >
                <span
                  className={`size-1.5 shrink-0 rounded-full ${current ? "bg-accent" : entry.onCurrentPath ? "bg-fg-subtle" : "border border-fg-subtle"}`}
                />
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                {snap && (
                  <span className="flex items-center gap-1 text-2xs text-accent-text">
                    <Bookmark className="size-3" />
                    {snap.name}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
