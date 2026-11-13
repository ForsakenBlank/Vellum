import { LifeBuoy } from "lucide-react";
import { AlertDialog } from "radix-ui";
import { useRef } from "react";

import { MediaIcon } from "../components/MediaIcon";
import { formatClock } from "../lib/format";
import { discardRecovery, restoreSession, useSession } from "../stores/session";

const SHOWN = 6;

/** asks whether to bring back what was open when vellum last closed unexpectedly */
export function RecoveryPrompt() {
  const offer = useSession((state) => state.recovery);
  const restore = useRef<HTMLButtonElement>(null);
  if (!offer) return null;
  const count = offer.documents.length;
  const extra = count - SHOWN;

  return (
    <AlertDialog.Root open>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="v-fade fixed inset-0 z-40 bg-black/55 backdrop-blur-[2px]" />
        <AlertDialog.Content
          // restoring loses nothing, so it gets the focus rather than "start fresh"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            restore.current?.focus();
          }}
          className="v-pop fixed top-[18vh] left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 rounded-xl border border-line bg-surface p-5 shadow-modal"
        >
          <div className="flex gap-3.5">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-accent-soft text-accent">
              <LifeBuoy className="size-5" aria-hidden />
            </div>
            <div className="min-w-0">
              <AlertDialog.Title className="text-base font-semibold">
                Restore your last session?
              </AlertDialog.Title>
              <AlertDialog.Description className="mt-1 text-sm text-ink-soft">
                Vellum closed unexpectedly. {count === 1 ? "One file was" : `${count} files were`}{" "}
                open when it last saved, at {formatClock(offer.savedMs)}.
              </AlertDialog.Description>
            </div>
          </div>

          <ul
            aria-label="Files to restore"
            className="mt-4 rounded-lg border border-line bg-canvas py-1"
          >
            {offer.documents.slice(0, SHOWN).map((doc) => (
              <li key={doc.id} className="flex items-center gap-2.5 px-3 py-1.5 text-sm">
                <MediaIcon kind={doc.kind} className="size-3.5 shrink-0 text-ink-soft" />
                <span className="truncate">{doc.title}</span>
              </li>
            ))}
            {extra > 0 && <li className="px-3 py-1.5 text-xs text-ink-soft">and {extra} more</li>}
          </ul>

          <div className="mt-5 flex justify-end gap-2">
            <AlertDialog.Cancel
              onClick={() => void discardRecovery()}
              className="v-focus inline-flex h-8 items-center rounded-md border border-line bg-raised px-3 text-sm font-medium hover:border-line-strong"
            >
              Start fresh
            </AlertDialog.Cancel>
            <AlertDialog.Action
              ref={restore}
              onClick={() => void restoreSession()}
              className="v-focus inline-flex h-8 items-center rounded-md bg-accent px-3 text-sm font-medium text-on-accent hover:brightness-110"
            >
              Restore
            </AlertDialog.Action>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
