import { Layers, Rows3, SlidersHorizontal, Sparkles, Wand } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const motionWorkspace: Workspace = {
  id: "motion",
  title: "Motion",
  icon: Sparkles,
  summary: "Animated shapes, text and compositing.",
  phase: 12,
  accepts: "videos and images",
  panels: [
    placeholderPanel("layers", "Layers", Layers, "Shapes, text and footage layers."),
    placeholderPanel("properties", "Properties", SlidersHorizontal, "Keyframes and settings."),
    placeholderPanel("effects", "Effects", Wand, "Effects and presets to drag on."),
    placeholderPanel("timeline", "Timeline", Rows3, "Keyframes over time."),
    jobsPanel,
  ],
  layout: dockLayout({
    left: [["layers"]],
    right: [["properties", "effects"]],
    bottom: [["timeline"]],
  }),
};
