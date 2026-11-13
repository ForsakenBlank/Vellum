import { ChevronDown } from "lucide-react";

interface SelectProps {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}

/** a native select, styled to match. native keeps keyboard and screen reader support for free */
export function Select({ label, value, options, onChange }: SelectProps) {
  return (
    <div className="relative">
      <select
        aria-label={label}
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        className="v-focus h-8 appearance-none rounded-md border border-line bg-raised pr-8 pl-2.5 text-sm text-ink hover:border-line-strong"
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown
        className="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-ink-soft"
        aria-hidden
      />
    </div>
  );
}
