import { Activity, CircleDot, GalleryHorizontal, Network, Palette, Spline } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const colorWorkspace: Workspace = {
  id: "color",
  title: "Color",
  icon: Palette,
  summary: "Wheels, curves, scopes and LUTs for grading.",
  phase: 10,
  accepts: "videos and images",
  panels: [
    placeholderPanel("gallery", "Gallery", GalleryHorizontal, "Saved grades to reuse."),
    placeholderPanel("nodes", "Nodes", Network, "The grade as a chain of nodes."),
    placeholderPanel("scopes", "Scopes", Activity, "Waveform, parade and vectorscope."),
    placeholderPanel("wheels", "Wheels", CircleDot, "Lift, gamma and gain."),
    placeholderPanel("curves", "Curves", Spline, "Curves per channel and hue against hue."),
    jobsPanel,
  ],
  layout: dockLayout({
    left: [["gallery"]],
    right: [["nodes"], ["scopes"]],
    bottom: [["wheels", "curves"]],
    sizes: { left: 12, bottom: 15 },
  }),
};
