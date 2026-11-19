import type { ComponentProps, ReactNode } from "react";

import { Tooltip } from "./Tooltip";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "default";
export type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps extends ComponentProps<"button"> {
  /** every button explains itself on hover */
  tooltip: string;
  shortcut?: string | undefined;
  /** shows the current shortcut of this command in the tooltip */
  command?: string | undefined;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
}

const variants: Record<ButtonVariant, string> = {
  primary:
    "bg-accent text-accent-fg font-medium shadow-1 hover:brightness-110 active:brightness-95",
  secondary: "bg-raised text-fg border border-line-strong hover:bg-elevated active:bg-pressed",
  default: "bg-raised text-fg border border-line-strong hover:bg-elevated active:bg-pressed",
  ghost: "text-fg-muted hover:bg-elevated hover:text-fg active:bg-pressed",
  danger: "bg-raised text-danger border border-line-strong hover:border-danger active:bg-pressed",
};

const sizes: Record<ButtonSize, string> = {
  sm: "h-(--row-sm) px-2 text-xs gap-1.5",
  md: "h-(--row) px-3 text-sm gap-2",
  lg: "h-10 px-4 text-base gap-2",
};

export function Button({
  tooltip,
  shortcut,
  command,
  variant = "secondary",
  size = "md",
  icon,
  className = "",
  children,
  ...rest
}: ButtonProps) {
  return (
    <Tooltip text={tooltip} shortcut={shortcut} command={command}>
      <button
        type="button"
        className={`inline-flex shrink-0 items-center justify-center rounded-md whitespace-nowrap transition-[background-color,color,border-color,filter] duration-(--motion-fast) disabled:pointer-events-none disabled:opacity-45 ${variants[variant]} ${sizes[size]} ${className}`}
        {...rest}
      >
        {icon}
        {children}
      </button>
    </Tooltip>
  );
}

interface IconButtonProps extends Omit<ComponentProps<"button">, "children"> {
  /** read out by screen readers and shown as the tooltip */
  label: string;
  icon: ReactNode;
  shortcut?: string | undefined;
  command?: string | undefined;
  active?: boolean;
  size?: "sm" | "md";
  side?: "top" | "right" | "bottom" | "left";
}

export function IconButton({
  label,
  icon,
  shortcut,
  command,
  active = false,
  size = "md",
  side,
  className = "",
  ...rest
}: IconButtonProps) {
  const box = size === "sm" ? "size-6" : "size-(--row)";
  return (
    <Tooltip text={label} shortcut={shortcut} command={command} side={side}>
      <button
        type="button"
        aria-label={label}
        aria-pressed={active || undefined}
        className={`inline-flex shrink-0 items-center justify-center rounded-md transition-colors duration-(--motion-fast) disabled:pointer-events-none disabled:opacity-40 ${
          active
            ? "bg-accent-soft text-accent-text"
            : "text-fg-muted hover:bg-elevated hover:text-fg active:bg-pressed"
        } ${box} ${className}`}
        {...rest}
      >
        {icon}
      </button>
    </Tooltip>
  );
}
