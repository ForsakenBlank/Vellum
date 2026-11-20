// small controls the quick tool views share. every one has a visible label and a tooltip
// that says what it does, and sliders end their undo step when the pointer lets go.

import { ChevronRight } from "lucide-react";
import { useId, useState, type ReactNode } from "react";

import { Switch } from "../../components/Switch";
import { Tooltip } from "../../components/Tooltip";

export function Section({
  title,
  aside,
  children,
}: {
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">{title}</h3>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** the "advanced" drawer: closed by default, full control one click away */
export function Advanced({
  children,
  label = "Advanced",
}: {
  children: ReactNode;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="flex flex-col gap-3">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
        className="flex items-center gap-1.5 self-start rounded-sm text-xs font-medium text-fg-muted hover:text-fg"
      >
        <ChevronRight
          className={`size-3.5 transition-transform duration-(--motion-fast) ${open ? "rotate-90" : ""}`}
        />
        {label}
      </button>
      {open && <div className="flex flex-col gap-4 border-l border-line pl-4">{children}</div>}
    </div>
  );
}

export function Field({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-xs font-medium text-fg-muted">
        {label}
      </label>
      {children}
      {hint && <p className="text-2xs text-fg-subtle">{hint}</p>}
    </div>
  );
}

export interface Choice<T extends string | number> {
  value: T;
  label: string;
  tooltip: string;
}

export function Segmented<T extends string | number>({
  label,
  value,
  choices,
  onChange,
}: {
  label: string;
  value: T;
  choices: Choice<T>[];
  onChange: (value: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1">
      {choices.map((choice) => {
        const on = choice.value === value;
        return (
          <Tooltip key={String(choice.value)} text={choice.tooltip}>
            <button
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => {
                onChange(choice.value);
              }}
              className={`h-(--row-sm) rounded-md border px-2.5 text-xs transition-colors duration-(--motion-fast) ${
                on
                  ? "border-accent-line bg-accent-soft font-medium text-accent-text"
                  : "border-line bg-raised text-fg-muted hover:border-line-strong hover:text-fg"
              }`}
            >
              {choice.label}
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}

export function Slider({
  label,
  value,
  min,
  max,
  step = 1,
  format = String,
  onChange,
  onCommit,
  hint,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (value: number) => string;
  onChange: (value: number) => void;
  /** the drag ended, so the next change is a new undo step */
  onCommit: () => void;
  hint?: ReactNode;
}) {
  const id = useId();
  return (
    <Field label={label} htmlFor={id} hint={hint}>
      <div className="flex items-center gap-3">
        <input
          id={id}
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(event) => {
            onChange(Number(event.target.value));
          }}
          onPointerUp={onCommit}
          onKeyUp={onCommit}
          className="h-1.5 min-w-0 flex-1 cursor-pointer accent-(--accent)"
        />
        <output htmlFor={id} className="w-16 shrink-0 text-right text-xs text-fg tabular">
          {format(value)}
        </output>
      </div>
    </Field>
  );
}

export function NumberField({
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  suffix,
  placeholder,
  hint,
  className = "w-24",
}: {
  label: string;
  value: number | null;
  /** null when the field is cleared */
  onChange: (value: number | null) => void;
  min?: number;
  max?: number;
  step?: number;
  suffix?: string;
  placeholder?: string;
  hint?: ReactNode;
  className?: string;
}) {
  const id = useId();
  return (
    <Field label={label} htmlFor={id} hint={hint}>
      <div className="flex items-center gap-1.5">
        <input
          id={id}
          type="number"
          inputMode="decimal"
          min={min}
          max={max}
          step={step}
          value={value ?? ""}
          placeholder={placeholder}
          onChange={(event) => {
            const text = event.target.value;
            if (text === "") {
              onChange(null);
              return;
            }
            const next = Number(text);
            if (Number.isFinite(next)) onChange(next);
          }}
          className={`h-(--row) rounded-md border border-line bg-raised px-2 text-sm text-fg tabular placeholder:text-fg-subtle focus:border-accent-line focus:outline-none ${className}`}
        />
        {suffix && <span className="text-xs text-fg-subtle">{suffix}</span>}
      </div>
    </Field>
  );
}

export function Toggle({
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-4">
      <label htmlFor={id} className="flex flex-col gap-0.5">
        <span className="text-sm text-fg">{label}</span>
        {hint && <span className="text-2xs text-fg-subtle">{hint}</span>}
      </label>
      <Switch
        id={id}
        label={label}
        checked={checked}
        onCheckedChange={onChange}
        disabled={disabled ?? false}
      />
    </div>
  );
}

export function TextField({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}) {
  const id = useId();
  return (
    <Field label={label} htmlFor={id}>
      <input
        id={id}
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        className="h-(--row) w-full rounded-md border border-line bg-raised px-2.5 text-sm text-fg placeholder:text-fg-subtle focus:border-accent-line focus:outline-none"
      />
    </Field>
  );
}

/** a short line of feedback: a plan, an estimate, a warning */
export function Note({
  tone = "muted",
  children,
}: {
  tone?: "muted" | "warning" | "danger" | "success";
  children: ReactNode;
}) {
  const tones = {
    muted: "bg-raised text-fg-muted",
    warning: "bg-warning/10 text-warning",
    danger: "bg-danger/10 text-danger",
    success: "bg-success/10 text-success",
  };
  return <p className={`rounded-md px-2.5 py-2 text-xs ${tones[tone]}`}>{children}</p>;
}
