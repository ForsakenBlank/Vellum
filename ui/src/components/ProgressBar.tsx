interface ProgressBarProps {
  /** 0 to 1, or null when the amount of work is unknown */
  value: number | null;
  label: string;
  tone?: "accent" | "muted" | "danger" | "success";
}

const tones = {
  accent: "bg-accent",
  muted: "bg-muted",
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
      className="h-1.5 w-full overflow-hidden rounded-full bg-raised"
    >
      <div
        className={`h-full rounded-full transition-[width] duration-150 ${tones[tone]} ${percent === null ? "w-1/3 animate-pulse" : ""}`}
        style={percent === null ? undefined : { width: `${percent}%` }}
      />
    </div>
  );
}
