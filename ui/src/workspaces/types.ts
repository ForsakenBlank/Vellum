import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

import type { DockLayout } from "../bindings/DockLayout";
import type { OpenDocument } from "../bindings/OpenDocument";

/** a panel that can be docked, tabbed and moved around */
export interface PanelDef {
  /** unique within a workspace, used in saved layouts */
  id: string;
  title: string;
  icon: LucideIcon;
  component: ComponentType;
}

export interface Workspace {
  id: string;
  title: string;
  icon: LucideIcon;
  /** one line on what the workspace is for */
  summary: string;
  /** the phase in PLAN.md that builds it, shown while it is a placeholder */
  phase: number;
  /** what can be dropped on it, for its empty state, such as "images" */
  accepts: string;
  /** every panel the workspace offers, docked or not */
  panels: PanelDef[];
  /** where the panels start out, and where "reset layout" goes back to */
  layout: DockLayout;
  /** missing until the phase that builds the workspace, the shell shows a placeholder */
  component?: ComponentType<{ document: OpenDocument }>;
}
