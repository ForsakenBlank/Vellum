import { clsx } from "clsx";
import { Progress } from "radix-ui";

interface ProgressBarProps {
  /** 0 to 1, or null when there is no telling how far along things are */
  value: number | null;
  label: string;
  tone?: "accent" | "good" | "bad" | "idle";
}

const tones = {
  accent: "bg-accent",
  good: "bg-good",
  bad: "bg-bad",
  idle: "bg-ink-soft",
};

export function ProgressBar({ value, label, tone = "accent" }: ProgressBarProps) {
  const percent = value === null ? null : Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <Progress.Root
      value={percent}
      aria-label={label}
      className="h-1 w-full overflow-hidden rounded-full bg-raised"
    >
      <Progress.Indicator
        className={clsx(
          "h-full rounded-full transition-[width] duration-150 ease-out",
          tones[tone],
          percent === null && "w-1/3 animate-pulse",
        )}
        style={percent === null ? undefined : { width: `${percent}%` }}
      />
    </Progress.Root>
  );
}
