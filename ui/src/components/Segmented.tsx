import { clsx } from "clsx";
import { ToggleGroup } from "radix-ui";
import type { ReactNode } from "react";

interface Option<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface SegmentedProps<T extends string> {
  label: string;
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
}

/** a small row of choices where exactly one is always picked */
export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: SegmentedProps<T>) {
  return (
    <ToggleGroup.Root
      type="single"
      aria-label={label}
      value={value}
      onValueChange={(next) => {
        // radix sends an empty string when the active item is clicked again
        const picked = options.find((option) => option.value === next);
        if (picked) onChange(picked.value);
      }}
      className="inline-flex rounded-md border border-line bg-raised p-0.5"
    >
      {options.map((option) => (
        <ToggleGroup.Item
          key={option.value}
          value={option.value}
          className={clsx(
            "inline-flex h-7 items-center gap-1.5 rounded px-2.5 text-sm text-ink-soft transition-colors",
            "outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent",
            "data-[state=on]:bg-surface data-[state=on]:text-ink data-[state=on]:shadow-sm",
          )}
        >
          {option.icon}
          {option.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
