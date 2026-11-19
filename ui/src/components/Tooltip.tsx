import { Tooltip as RadixTooltip } from "radix-ui";
import type { ReactNode } from "react";

import { useCommands } from "../state/commands";
import { Kbd } from "./Kbd";

interface TooltipProps {
  text: string;
  /** a chord like "Ctrl+K", shown as key caps after the text */
  shortcut?: string | undefined;
  /** a command kind whose current shortcut should be shown */
  command?: string | undefined;
  side?: "top" | "right" | "bottom" | "left" | undefined;
  children: ReactNode;
}

export function Tooltip({ text, shortcut, command, side, children }: TooltipProps) {
  const bound = useCommands((s) => (command ? s.keymap?.bindings[command]?.[0] : undefined));
  const chord = shortcut ?? bound;
  return (
    <RadixTooltip.Root>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <RadixTooltip.Content
          side={side ?? "bottom"}
          sideOffset={6}
          collisionPadding={8}
          className="z-[var(--z-tooltip)] flex max-w-72 animate-fade-in items-center gap-2 rounded-md bg-elevated px-2 py-1 text-xs text-fg shadow-pop"
        >
          <span>{text}</span>
          {chord && <Kbd chord={chord} />}
        </RadixTooltip.Content>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}

export const TooltipProvider = RadixTooltip.Provider;
