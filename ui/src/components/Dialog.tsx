import { Dialog as RadixDialog } from "radix-ui";
import type { ReactNode } from "react";

interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  width?: string;
  /** hides the title visually but keeps it for screen readers */
  bare?: boolean;
  /** where the dialog sits: centered, or near the top like a palette */
  placement?: "center" | "top";
  className?: string;
}

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  width = "32rem",
  bare = false,
  placement = "center",
  className = "",
}: DialogProps) {
  return (
    <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="fixed inset-0 z-[var(--z-modal)] animate-fade-in bg-scrim backdrop-blur-[2px]" />
        <RadixDialog.Content
          // radix links the description itself; without one, say so to keep it quiet
          {...(description ? {} : { "aria-describedby": undefined })}
          style={{ width: `min(${width}, calc(100vw - 2rem))` }}
          className={`fixed left-1/2 z-[var(--z-modal)] flex max-h-[calc(100vh-4rem)] -translate-x-1/2 animate-pop-in flex-col overflow-hidden rounded-xl bg-panel text-fg shadow-pop ${
            placement === "top" ? "top-[12vh]" : "top-1/2 -translate-y-1/2"
          } ${className}`}
        >
          {bare ? (
            <RadixDialog.Title className="sr-only">{title}</RadixDialog.Title>
          ) : (
            <div className="flex flex-col gap-1 px-5 pt-5">
              <RadixDialog.Title className="text-md font-semibold tracking-tight">
                {title}
              </RadixDialog.Title>
              {description && (
                <RadixDialog.Description className="text-sm text-fg-muted">
                  {description}
                </RadixDialog.Description>
              )}
            </div>
          )}
          {bare && description && (
            <RadixDialog.Description className="sr-only">{description}</RadixDialog.Description>
          )}
          {children}
          {footer && (
            <div className="flex items-center justify-end gap-2 border-t border-line bg-canvas/40 px-5 py-3">
              {footer}
            </div>
          )}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
