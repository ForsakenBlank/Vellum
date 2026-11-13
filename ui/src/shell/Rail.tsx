import { clsx } from "clsx";
import { House } from "lucide-react";

import { useKeymap } from "../commands/registry";
import { IconButton } from "../components/Button";
import { shell, useWorkspaceId } from "../stores/session";
import { workspaces } from "../workspaces";
import { useDrop } from "./drop";

/**
 * the strip down the left edge for moving between workspaces. each icon is
 * also a drop target: a file dropped on the video icon opens in video
 */
export function Rail() {
  const active = useWorkspaceId();
  const keymap = useKeymap();
  const dropKey = useDrop((state) => state.targetKey);

  return (
    <nav
      aria-label="Workspaces"
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-line bg-surface py-2"
    >
      <IconButton
        label="Home"
        shortcut={keymap["shell.home"]?.[0]}
        tipSide="right"
        active={active === null}
        aria-current={active === null ? "page" : undefined}
        className="size-9"
        onClick={() => void shell.home()}
      >
        <House className="size-5" strokeWidth={1.75} />
      </IconButton>
      <div className="my-1 h-px w-6 bg-line" />
      {workspaces.map((workspace) => {
        const key = `workspace:${workspace.id}`;
        const selected = active === workspace.id;
        return (
          <div key={workspace.id} data-drop={key} className="relative">
            {selected && (
              <span className="absolute top-1/2 -left-1.5 h-5 w-0.5 -translate-y-1/2 rounded-r bg-accent" />
            )}
            <IconButton
              label={workspace.title}
              shortcut={keymap[`go.${workspace.id}`]?.[0]}
              tipSide="right"
              active={selected}
              aria-current={selected ? "page" : undefined}
              className={clsx("size-9", dropKey === key && "bg-accent text-on-accent")}
              onClick={() => void shell.workspace(workspace.id)}
            >
              <workspace.icon className="size-5" strokeWidth={1.75} />
            </IconButton>
          </div>
        );
      })}
    </nav>
  );
}
