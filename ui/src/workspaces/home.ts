import { House } from "lucide-react";

import { dockLayout, jobsPanel } from "../dock/panels";
import type { Workspace } from "./types";

/**
 * the home screen is not a workspace people pick from the rail, but it docks
 * panels like one, so it has the same shape
 */
export const homeWorkspace: Workspace = {
  id: "home",
  title: "Home",
  icon: House,
  summary: "Recent files, pinned projects and quick tools.",
  phase: 1,
  accepts: "any media file",
  panels: [jobsPanel],
  layout: dockLayout({ right: [["jobs"]] }),
};
