import type { ComponentProps } from "react";

import { Tooltip } from "./Tooltip";

type Variant = "default" | "primary" | "ghost" | "danger";

interface ButtonProps extends ComponentProps<"button"> {
  /** every button explains itself on hover */
  tooltip: string;
  shortcut?: string | undefined;
  variant?: Variant;
}

const variants: Record<Variant, string> = {
  default: "border border-line bg-raised hover:border-muted",
  primary: "bg-accent text-white hover:brightness-110",
  ghost: "text-muted hover:bg-raised hover:text-text",
  danger: "border border-line bg-raised text-danger hover:border-danger",
};

export function Button({
  tooltip,
  shortcut,
  variant = "default",
  className = "",
  ...rest
}: ButtonProps) {
  return (
    <Tooltip text={tooltip} shortcut={shortcut}>
      <button
        type="button"
        className={`inline-flex items-center justify-center gap-1 rounded-md px-2.5 py-1 text-sm transition-colors duration-100 focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50 ${variants[variant]} ${className}`}
        {...rest}
      />
    </Tooltip>
  );
}
