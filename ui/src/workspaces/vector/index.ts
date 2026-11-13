import { Files, Layers, PenTool, SlidersHorizontal, SwatchBook } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const vectorWorkspace: Workspace = {
  id: "vector",
  title: "Vector",
  icon: PenTool,
  summary: "Paths, shapes, PDFs and clean SVG export.",
  phase: 15,
  accepts: "SVGs and PDFs",
  panels: [
    placeholderPanel("layers", "Layers", Layers, "Paths, groups and shapes."),
    placeholderPanel("pages", "Pages", Files, "Pages of a PDF or artboards."),
    placeholderPanel("properties", "Properties", SlidersHorizontal, "Fill, stroke and size."),
    placeholderPanel("swatches", "Swatches", SwatchBook, "Document colours."),
    jobsPanel,
  ],
  layout: dockLayout({
    left: [["layers", "pages"]],
    right: [["properties", "swatches"]],
  }),
};
