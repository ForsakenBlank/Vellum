import { GitBranch, Info, ListChecks, type LucideIcon } from "lucide-react";

import type { AreaName } from "../../lib/dock";

export interface PanelInfo {
  id: string;
  title: string;
  icon: LucideIcon;
  /** the dock a panel goes back to when it was closed and is shown again */
  home: AreaName;
  /** the app command that shows it */
  command: string;
}

export const panels: PanelInfo[] = [
  { id: "info", title: "Info", icon: Info, home: "right", command: "view.panel.info" },
  {
    id: "history",
    title: "History",
    icon: GitBranch,
    home: "right",
    command: "view.panel.history",
  },
  { id: "jobs", title: "Jobs", icon: ListChecks, home: "right", command: "view.panel.jobs" },
];

export const panelIds = panels.map((p) => p.id);

export function panelInfo(id: string): PanelInfo | undefined {
  return panels.find((p) => p.id === id);
}
