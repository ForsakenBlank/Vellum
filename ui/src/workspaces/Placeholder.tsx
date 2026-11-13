import type { Workspace } from "./types";

/** stands in for a workspace until its phase is built */
export function Placeholder({ workspace }: { workspace: Workspace }) {
  const Icon = workspace.icon;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
      <Icon className="size-10 text-ink-soft" strokeWidth={1.5} aria-hidden />
      <h1 className="text-lg font-semibold">{workspace.title}</h1>
      <p className="max-w-sm text-sm text-ink-soft">{workspace.summary}</p>
      <p className="text-xs text-ink-soft">This workspace is not built yet.</p>
    </div>
  );
}
