import { useState } from "react";

import { workspaces } from "../workspaces";
import { Placeholder } from "../workspaces/Placeholder";
import { Home } from "./Home";
import { JobsPanel } from "./JobsPanel";
import { Notice } from "./Notice";
import { Rail } from "./Rail";

export function AppShell() {
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = workspaces.find((workspace) => workspace.id === activeId);

  return (
    <div className="flex h-full flex-col bg-canvas text-ink">
      <Notice />
      <div className="flex min-h-0 flex-1">
        <Rail workspaces={workspaces} active={active?.id ?? null} onSelect={setActiveId} />
        <main className="min-w-0 flex-1">
          {!active ? (
            <Home />
          ) : active.component ? (
            <active.component />
          ) : (
            <Placeholder workspace={active} />
          )}
        </main>
        <JobsPanel />
      </div>
    </div>
  );
}
