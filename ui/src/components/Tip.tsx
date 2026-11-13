import { Tooltip } from "radix-ui";
import type { ReactElement } from "react";

interface TipProps {
  label: string;
  /** shown after the label, such as "Ctrl+K" */
  shortcut?: string;
  children: ReactElement;
}

/** a tooltip. every button gets one, with its shortcut when it has one */
export function Tip({ label, shortcut, children }: TipProps) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          sideOffset={6}
          className="z-50 flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-xs text-ink shadow-lg"
        >
          {label}
          {shortcut && <kbd className="font-sans text-ink-soft">{shortcut}</kbd>}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
