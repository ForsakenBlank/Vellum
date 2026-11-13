import { clsx } from "clsx";

import { formatShortcut } from "../lib/keys";

/** a shortcut drawn as keys, in the form this platform writes them */
export function Kbd({ shortcut, className }: { shortcut: string; className?: string }) {
  return (
    <kbd
      className={clsx(
        "inline-flex h-5 min-w-5 items-center justify-center rounded border border-line bg-canvas px-1.5",
        "font-sans text-[0.6875rem] leading-none font-medium text-ink-soft",
        className,
      )}
    >
      {formatShortcut(shortcut)}
    </kbd>
  );
}
