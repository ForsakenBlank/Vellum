// "3s", "2m 05s", "1h 02m", or null when there is nothing useful to say
export function formatEta(secs: number | null): string | null {
  if (secs === null || !Number.isFinite(secs) || secs < 0) return null;
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}m`;
}

export function formatPercent(progress: number): string {
  return `${Math.floor(Math.min(Math.max(progress, 0), 1) * 100)}%`;
}
