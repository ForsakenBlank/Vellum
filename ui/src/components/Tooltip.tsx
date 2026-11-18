import { Tooltip as RadixTooltip } from "radix-ui";
import type { ReactNode } from "react";

interface TooltipProps {
  text: string;
  /** shown after the text, eg "Ctrl+K" */
  shortcut?: string | undefined;
  children: ReactNode;
}

export function Tooltip({ text, shortcut, children }: TooltipProps) {
  return (
    <RadixTooltip.Root>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <RadixTooltip.Content
          side="bottom"
          sideOffset={6}
          className="z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-xs text-text shadow-lg"
        >
          {text}
          {shortcut && (
            <kbd className="rounded border border-line px-1 font-sans text-[0.7rem] text-muted">
              {shortcut}
            </kbd>
          )}
        </RadixTooltip.Content>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}

export const TooltipProvider = RadixTooltip.Provider;
