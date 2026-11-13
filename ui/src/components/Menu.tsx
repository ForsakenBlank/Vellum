import { clsx } from "clsx";
import { Check, ChevronRight } from "lucide-react";
import { ContextMenu, DropdownMenu } from "radix-ui";
import type { ReactNode } from "react";

import { Kbd } from "./Kbd";

const content =
  "v-pop z-50 min-w-48 overflow-hidden rounded-lg border border-line bg-raised p-1 text-sm text-ink shadow-pop";
const item =
  "flex h-7 cursor-default items-center gap-2 rounded px-2 outline-none select-none data-[disabled]:opacity-40 data-[highlighted]:bg-accent data-[highlighted]:text-on-accent";

interface ItemProps {
  children: ReactNode;
  onSelect: () => void;
  shortcut?: string | null;
  disabled?: boolean;
  icon?: ReactNode;
  /** draws a tick, for toggles */
  checked?: boolean;
}

function ItemBody({ children, shortcut, icon, checked }: Omit<ItemProps, "onSelect">) {
  return (
    <>
      <span className="flex size-4 shrink-0 items-center justify-center">
        {checked ? <Check className="size-3.5" aria-hidden /> : icon}
      </span>
      <span className="flex-1 truncate">{children}</span>
      {shortcut && <Kbd shortcut={shortcut} className="border-transparent bg-transparent" />}
    </>
  );
}

/** a menu that opens from a button */
export const Menu = {
  Root: DropdownMenu.Root,
  Trigger: DropdownMenu.Trigger,
  Content({ children, align = "start" }: { children: ReactNode; align?: "start" | "end" }) {
    return (
      <DropdownMenu.Portal>
        <DropdownMenu.Content align={align} sideOffset={4} className={content}>
          {children}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    );
  },
  Item({ onSelect, disabled, checked, ...rest }: ItemProps) {
    const props = { ...rest, checked };
    return checked === undefined ? (
      <DropdownMenu.Item className={item} disabled={disabled} onSelect={onSelect}>
        <ItemBody {...props} />
      </DropdownMenu.Item>
    ) : (
      <DropdownMenu.CheckboxItem
        className={item}
        disabled={disabled}
        checked={checked}
        onSelect={(event) => {
          // a toggle keeps the menu open so several can be flipped in a row
          event.preventDefault();
          onSelect();
        }}
      >
        <ItemBody {...props} />
      </DropdownMenu.CheckboxItem>
    );
  },
  Label({ children }: { children: ReactNode }) {
    return (
      <DropdownMenu.Label className="px-2 pt-1.5 pb-1 text-[0.6875rem] font-medium tracking-wide text-ink-soft uppercase">
        {children}
      </DropdownMenu.Label>
    );
  },
  Separator() {
    return <DropdownMenu.Separator className="my-1 h-px bg-line" />;
  },
  /** a nested menu that opens to the side */
  Sub({ label, icon, children }: { label: string; icon?: ReactNode; children: ReactNode }) {
    return (
      <DropdownMenu.Sub>
        <DropdownMenu.SubTrigger className={clsx(item, "data-[state=open]:bg-hover")}>
          <ItemBody icon={icon}>{label}</ItemBody>
          <ChevronRight className="size-3.5 text-ink-soft" aria-hidden />
        </DropdownMenu.SubTrigger>
        <DropdownMenu.Portal>
          <DropdownMenu.SubContent sideOffset={4} className={content}>
            {children}
          </DropdownMenu.SubContent>
        </DropdownMenu.Portal>
      </DropdownMenu.Sub>
    );
  },
};

/** a menu that opens on right click */
export const RightClickMenu = {
  Root: ContextMenu.Root,
  Trigger: ContextMenu.Trigger,
  Content({ children }: { children: ReactNode }) {
    return (
      <ContextMenu.Portal>
        <ContextMenu.Content className={content}>{children}</ContextMenu.Content>
      </ContextMenu.Portal>
    );
  },
  Item({ onSelect, disabled, ...rest }: ItemProps) {
    return (
      <ContextMenu.Item className={clsx(item)} disabled={disabled} onSelect={onSelect}>
        <ItemBody {...rest} />
      </ContextMenu.Item>
    );
  },
  Separator() {
    return <ContextMenu.Separator className="my-1 h-px bg-line" />;
  },
};
