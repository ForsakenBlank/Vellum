import { Film, FolderOpen, History, Rows3, SlidersHorizontal, Wand } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const videoWorkspace: Workspace = {
  id: "video",
  title: "Video",
  icon: Film,
  summary: "A multi track timeline for cutting, effects and titles.",
  phase: 8,
  accepts: "videos, images and audio",
  panels: [
    placeholderPanel("media", "Media", FolderOpen, "Clips you bring into the project live here."),
    placeholderPanel("effects", "Effects", Wand, "Transitions, titles and effects to drag on."),
    placeholderPanel(
      "inspector",
      "Inspector",
      SlidersHorizontal,
      "Settings for the selected clip show here.",
    ),
    placeholderPanel("timeline", "Timeline", Rows3, "The multi track timeline arrives in phase 8."),
    placeholderPanel("history", "History", History, "Every edit, with snapshots."),
    jobsPanel,
  ],
  layout: dockLayout({
    left: [["media", "effects"]],
    right: [["inspector", "history"]],
    bottom: [["timeline"]],
    sizes: { bottom: 16 },
  }),
};
