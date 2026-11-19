import * as RadixTooltip from "@radix-ui/react-tooltip";
import type { ReactNode } from "react";

interface TooltipProps {
  label: string;
  shortcut?: string;
  children: ReactNode;
}

// every button gets one of these (PLAN.md section 5: tooltips show the shortcut)
export function Tooltip({ label, shortcut, children }: TooltipProps) {
  return (
    <RadixTooltip.Root>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <RadixTooltip.Content
          sideOffset={6}
          className="z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-xs text-fg shadow-lg"
        >
          {label}
          {shortcut && <kbd className="font-sans text-muted">{shortcut}</kbd>}
        </RadixTooltip.Content>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}

export const TooltipProvider = RadixTooltip.Provider;
