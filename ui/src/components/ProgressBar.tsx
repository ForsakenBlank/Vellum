interface ProgressBarProps {
  /** 0 to 1, or null when the amount of work is unknown */
  value: number | null;
  label: string;
  tone?: "accent" | "muted" | "danger" | "success";
}

const tones = {
  accent: "bg-accent",
  muted: "bg-fg-subtle",
  danger: "bg-danger",
  success: "bg-success",
};

export function ProgressBar({ value, label, tone = "accent" }: ProgressBarProps) {
  const percent = value === null ? null : Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      className="h-1 w-full overflow-hidden rounded-full bg-pressed"
    >
      <div
        className={`h-full rounded-full transition-[width] duration-(--motion-base) ease-calm ${tones[tone]} ${percent === null ? "skeleton w-full" : ""}`}
        style={percent === null ? undefined : { width: `${percent}%` }}
      />
    </div>
  );
}
