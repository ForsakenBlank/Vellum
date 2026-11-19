import { LifeBuoy } from "lucide-react";
import { useState } from "react";

import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { formatAgo } from "../lib/format";
import { ipc } from "../lib/ipc";
import { useDocuments } from "../state/documents";
import { useUi } from "../state/ui";
import { workspaceInfo } from "../workspaces/workspaces";

/** asked once on launch when the last session left work behind */
export function RecoveryPrompt() {
  const offer = useUi((s) => s.recovery);
  const setRecovery = useUi((s) => s.setRecovery);
  const [busy, setBusy] = useState(false);
  if (!offer) return null;

  const crashed = offer.reason === "crash";
  const unsaved = offer.documents.filter((d) => d.dirty).length;

  const restore = async () => {
    setBusy(true);
    try {
      const restored = await ipc.restoreSession();
      const first = restored[0];
      if (first) useDocuments.getState().activate(first.id);
      useUi.getState().toast({
        level: "success",
        title:
          restored.length === 1 ? "Restored 1 document" : `Restored ${restored.length} documents`,
        body:
          unsaved > 0
            ? "Changes that were not saved are back. Save them when you are ready."
            : undefined,
      });
      if (restored.length < offer.documents.length) {
        useUi.getState().toast({
          level: "warning",
          title: "Some documents could not be restored",
          body: "Their recovery files were kept. Try again after the next update.",
        });
      }
    } catch (err) {
      useUi
        .getState()
        .toast({ level: "error", title: "The session could not be restored", body: String(err) });
    } finally {
      setBusy(false);
      setRecovery(null);
    }
  };

  const discard = async () => {
    setBusy(true);
    try {
      await ipc.discardRecovery();
    } finally {
      setBusy(false);
      setRecovery(null);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={() => {
        // the user has to choose, closing would leave the files in limbo
      }}
      title={crashed ? "Restore your last session?" : "Pick up where you left off?"}
      description={
        crashed
          ? `Vellum did not close properly. Your work was autosaved ${formatAgo(offer.savedAtMs)}.`
          : `Vellum was closed with unsaved changes ${formatAgo(offer.savedAtMs)}.`
      }
      width="30rem"
      initialFocus="[data-default-action]"
      footer={
        <>
          <Button
            variant="ghost"
            tooltip="Delete the autosaved copies. The original files are not touched."
            disabled={busy}
            onClick={() => void discard()}
          >
            Discard
          </Button>
          <Button
            data-default-action
            variant="primary"
            tooltip="Open every document again, with its unsaved changes"
            disabled={busy}
            onClick={() => void restore()}
          >
            Restore{" "}
            {offer.documents.length === 1 ? "document" : `${offer.documents.length} documents`}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 px-5 py-4">
        <ul
          aria-label="Documents to restore"
          className="flex flex-col gap-1 rounded-lg border border-line bg-canvas/40 p-1.5"
        >
          {offer.documents.map((doc, i) => {
            const space = workspaceInfo(doc.workspace);
            return (
              <li
                key={`${doc.title}-${i}`}
                className="flex items-center gap-2.5 rounded-md px-2 py-1.5"
              >
                <space.icon className="size-4 shrink-0 text-fg-subtle" strokeWidth={1.7} />
                <span className="min-w-0 flex-1 truncate text-sm" title={doc.path ?? doc.title}>
                  {doc.title}
                </span>
                {doc.dirty && (
                  <span className="shrink-0 rounded-sm bg-accent-soft px-1.5 py-0.5 text-2xs font-medium text-accent-text">
                    Unsaved changes
                  </span>
                )}
              </li>
            );
          })}
        </ul>
        <p className="flex items-start gap-2 text-xs text-fg-subtle">
          <LifeBuoy className="mt-0.5 size-3.5 shrink-0" />
          Autosave keeps a copy of every open document. Your original files were never changed.
        </p>
      </div>
    </Dialog>
  );
}
