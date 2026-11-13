import { clsx } from "clsx";
import { X } from "lucide-react";
import { Dialog as Radix } from "radix-ui";
import type { ReactNode } from "react";

import { IconButton } from "./Button";

interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** read by screen readers, shown under the title when `showDescription` is set */
  description: string;
  showDescription?: boolean;
  /** hides the title bar, for the command palette */
  bare?: boolean;
  className?: string;
  children: ReactNode;
}

/** a centred modal window over a dimmed app */
export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  showDescription,
  bare,
  className,
  children,
}: DialogProps) {
  return (
    <Radix.Root open={open} onOpenChange={onOpenChange}>
      <Radix.Portal>
        <Radix.Overlay className="v-fade fixed inset-0 z-40 bg-black/50 backdrop-blur-[2px]" />
        <Radix.Content
          className={clsx(
            "v-pop fixed top-[12vh] left-1/2 z-50 flex max-h-[76vh] w-[min(40rem,calc(100vw-2rem))] -translate-x-1/2 flex-col overflow-hidden",
            "rounded-xl border border-line bg-surface shadow-modal outline-none",
            className,
          )}
        >
          {bare ? (
            <>
              <Radix.Title className="sr-only">{title}</Radix.Title>
              <Radix.Description className="sr-only">{description}</Radix.Description>
            </>
          ) : (
            <header className="flex shrink-0 items-start gap-3 border-b border-line px-5 py-3.5">
              <div className="min-w-0 flex-1">
                <Radix.Title className="text-base font-semibold">{title}</Radix.Title>
                <Radix.Description
                  className={showDescription ? "mt-0.5 text-sm text-ink-soft" : "sr-only"}
                >
                  {description}
                </Radix.Description>
              </div>
              <Radix.Close asChild>
                <IconButton label="Close" shortcut="Escape">
                  <X className="size-4" />
                </IconButton>
              </Radix.Close>
            </header>
          )}
          {children}
        </Radix.Content>
      </Radix.Portal>
    </Radix.Root>
  );
}
