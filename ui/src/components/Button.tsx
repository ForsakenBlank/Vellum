import { clsx } from "clsx";
import type { ButtonHTMLAttributes } from "react";

import { Tip } from "./Tip";

const focusRing =
  "outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-canvas";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "quiet";
  /** tooltip text. leave it out only when the label already says everything */
  tip?: string;
  shortcut?: string;
}

export function Button({
  variant = "quiet",
  tip,
  shortcut,
  className,
  children,
  ...rest
}: ButtonProps) {
  const button = (
    <button
      type="button"
      className={clsx(
        "inline-flex h-8 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors",
        "disabled:pointer-events-none disabled:opacity-50",
        focusRing,
        variant === "primary"
          ? "bg-accent text-on-accent hover:brightness-110"
          : "border border-line bg-raised text-ink hover:border-ink-soft",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
  return tip ? (
    <Tip label={tip} shortcut={shortcut}>
      {button}
    </Tip>
  ) : (
    button
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** names the button for the tooltip and for screen readers */
  label: string;
  shortcut?: string;
  active?: boolean;
}

/** a square button that only shows an icon, so the label lives in its tooltip */
export function IconButton({
  label,
  shortcut,
  active,
  className,
  children,
  ...rest
}: IconButtonProps) {
  return (
    <Tip label={label} shortcut={shortcut}>
      <button
        type="button"
        aria-label={label}
        className={clsx(
          "inline-flex size-7 shrink-0 items-center justify-center rounded-md transition-colors",
          "disabled:pointer-events-none disabled:opacity-40",
          focusRing,
          active ? "bg-raised text-ink" : "text-ink-soft hover:bg-raised hover:text-ink",
          className,
        )}
        {...rest}
      >
        {children}
      </button>
    </Tip>
  );
}
