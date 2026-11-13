import { Bookmark, Zap } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const quickToolsWorkspace: Workspace = {
  id: "quick-tools",
  title: "Quick Tools",
  icon: Zap,
  summary: "Shrink a file to a target size, turn a clip into a GIF, or convert between formats.",
  phase: 3,
  accepts: "any media file",
  panels: [
    placeholderPanel("presets", "Presets", Bookmark, "Saved settings for each tool."),
    jobsPanel,
  ],
  layout: dockLayout({ right: [["jobs"], ["presets"]] }),
};
