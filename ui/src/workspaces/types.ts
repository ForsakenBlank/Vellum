import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

export interface Workspace {
  id: string;
  title: string;
  icon: LucideIcon;
  /** one line on what the workspace is for */
  summary: string;
  /** missing until the phase that builds the workspace, the shell shows a placeholder */
  component?: ComponentType;
}
