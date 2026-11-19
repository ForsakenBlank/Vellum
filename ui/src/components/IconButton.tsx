import type { ButtonHTMLAttributes } from "react";
import { Tooltip } from "./Tooltip";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  shortcut?: string;
}

export function IconButton({
  label,
  shortcut,
  className = "",
  children,
  ...rest
}: IconButtonProps) {
  return (
    <Tooltip label={label} shortcut={shortcut}>
      <button
        type="button"
        aria-label={label}
        className={`grid h-7 w-7 place-items-center rounded-md text-muted transition-colors duration-(--motion-fast) hover:bg-raised hover:text-fg focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 ${className}`}
        {...rest}
      >
        {children}
      </button>
    </Tooltip>
  );
}
