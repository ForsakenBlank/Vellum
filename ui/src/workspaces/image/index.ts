import { History, Image, Layers, SlidersHorizontal, SwatchBook } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const imageWorkspace: Workspace = {
  id: "image",
  title: "Image",
  icon: Image,
  summary: "Layers, selections, brushes, adjustments and filters.",
  phase: 5,
  accepts: "images",
  panels: [
    placeholderPanel("layers", "Layers", Layers, "Layers arrive with the image editor."),
    placeholderPanel(
      "properties",
      "Properties",
      SlidersHorizontal,
      "Settings for the selected layer or tool show here.",
    ),
    placeholderPanel("swatches", "Swatches", SwatchBook, "Saved colours arrive with the brushes."),
    placeholderPanel("history", "History", History, "Every change to the image, with snapshots."),
    jobsPanel,
  ],
  layout: dockLayout({
    right: [["layers", "swatches"], ["properties"], ["history"]],
  }),
};
