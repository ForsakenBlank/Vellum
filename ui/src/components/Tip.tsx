import { Tooltip } from "radix-ui";
import type { ReactElement } from "react";

import { formatShortcut } from "../lib/keys";

interface TipProps {
  label: string;
  /** shown after the label, such as "Mod+K" */
  shortcut?: string | null;
  side?: "top" | "right" | "bottom" | "left";
  children: ReactElement;
}

/** a tooltip. every button gets one, with its shortcut when it has one */
export function Tip({ label, shortcut, side, children }: TipProps) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side={side}
          sideOffset={6}
          className="v-pop z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-xs text-ink shadow-pop"
        >
          {label}
          {shortcut && <kbd className="font-sans text-ink-soft">{formatShortcut(shortcut)}</kbd>}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
