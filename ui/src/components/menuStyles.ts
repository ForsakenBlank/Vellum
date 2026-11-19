// shared classes for menubar, dropdown and context menus, so every menu looks the same

export const menuContent =
  "z-[var(--z-popover)] min-w-56 animate-pop-in rounded-lg bg-elevated p-1 text-sm text-fg shadow-pop";

export const menuItem =
  "flex h-(--row-sm) cursor-default items-center gap-2 rounded-sm px-2 outline-none select-none data-[disabled]:opacity-40 data-[highlighted]:bg-accent data-[highlighted]:text-accent-fg";

export const menuSeparator = "my-1 h-px bg-line";

export const menuLabel =
  "px-2 pt-1.5 pb-1 text-2xs font-medium tracking-wide text-fg-subtle uppercase";
