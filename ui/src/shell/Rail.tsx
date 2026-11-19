import { House } from "lucide-react";

import { Tooltip } from "../components/Tooltip";
import { activeDocument, useDocuments } from "../state/documents";
import { workspaces } from "../workspaces/workspaces";

/** the narrow strip on the left for jumping between home and the workspaces */
export function Rail() {
  const activeWorkspace = useDocuments((s) => activeDocument(s)?.workspace ?? s.emptyWorkspace);
  const homeActive = useDocuments((s) => s.activeId === null && s.emptyWorkspace === null);
  const showHome = useDocuments((s) => s.showHome);
  const showWorkspace = useDocuments((s) => s.showWorkspace);

  const item = (active: boolean) =>
    `relative flex size-9 items-center justify-center rounded-lg transition-colors duration-(--motion-fast) ${
      active ? "bg-raised text-fg shadow-1" : "text-fg-subtle hover:bg-raised/60 hover:text-fg"
    }`;

  return (
    <nav
      aria-label="Workspaces"
      className="flex w-(--rail) shrink-0 flex-col items-center gap-1 border-r border-line bg-panel py-2"
    >
      <Tooltip text="Home" command="app.home" side="right">
        <button
          type="button"
          aria-label="Home"
          aria-current={homeActive || undefined}
          className={item(homeActive)}
          onClick={showHome}
        >
          <House className="size-[1.05rem]" strokeWidth={1.7} />
        </button>
      </Tooltip>
      <span className="my-1 h-px w-5 bg-line" />
      {workspaces.map((ws) => {
        const active = !homeActive && activeWorkspace === ws.id;
        return (
          <Tooltip key={ws.id} text={`${ws.label}: ${ws.blurb}`} command={ws.command} side="right">
            <button
              type="button"
              aria-label={ws.label}
              aria-current={active || undefined}
              className={item(active)}
              onClick={() => {
                showWorkspace(ws.id);
              }}
            >
              {active && <span className="absolute -left-2 h-4 w-0.5 rounded-full bg-accent" />}
              <ws.icon className="size-[1.05rem]" strokeWidth={1.7} />
            </button>
          </Tooltip>
        );
      })}
    </nav>
  );
}
