import { House } from "lucide-react";

import { IconButton } from "../components/Button";
import type { Workspace } from "../workspaces";

interface RailProps {
  workspaces: Workspace[];
  /** null means the home page */
  active: string | null;
  onSelect: (id: string | null) => void;
}

/** the strip down the left edge for moving between workspaces */
export function Rail({ workspaces, active, onSelect }: RailProps) {
  return (
    <nav
      aria-label="Workspaces"
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-line bg-surface py-2"
    >
      <IconButton
        label="Home"
        active={active === null}
        aria-current={active === null ? "page" : undefined}
        className="size-9"
        onClick={() => {
          onSelect(null);
        }}
      >
        <House className="size-5" strokeWidth={1.75} />
      </IconButton>
      <div className="my-1 h-px w-6 bg-line" />
      {workspaces.map((workspace) => (
        <IconButton
          key={workspace.id}
          label={workspace.title}
          active={active === workspace.id}
          aria-current={active === workspace.id ? "page" : undefined}
          className="size-9"
          onClick={() => {
            onSelect(workspace.id);
          }}
        >
          <workspace.icon className="size-5" strokeWidth={1.75} />
        </IconButton>
      ))}
    </nav>
  );
}
