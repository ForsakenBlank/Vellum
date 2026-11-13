import { Eraser, FileDown, Film, MonitorDot, RefreshCw, type LucideIcon } from "lucide-react";

export interface QuickTool {
  /** matches `quick_tool_accepts` in crates/vellum-core/src/shell/files.rs */
  id: string;
  title: string;
  description: string;
  icon: LucideIcon;
  /** what the drop overlay says when files hover over the card */
  dropText: string;
  /** false for tools that do not take files, like the screen recorder */
  takesFiles: boolean;
  /** the phase in PLAN.md that builds it */
  phase: number;
}

export const quickTools: QuickTool[] = [
  {
    id: "export-to-size",
    title: "Export to 10 MB",
    description: "Shrink a video, image or song to fit an upload limit.",
    icon: FileDown,
    dropText: "Shrink to fit a size",
    takesFiles: true,
    phase: 3,
  },
  {
    id: "video-to-gif",
    title: "Video to GIF",
    description: "Turn a clip into a small, sharp GIF.",
    icon: Film,
    dropText: "Make a GIF",
    takesFiles: true,
    phase: 3,
  },
  {
    id: "remove-background",
    title: "Remove background",
    description: "Cut the subject out of a photo, on your own computer.",
    icon: Eraser,
    dropText: "Remove the background",
    takesFiles: true,
    phase: 7,
  },
  {
    id: "convert",
    title: "Convert",
    description: "Change any file to another format.",
    icon: RefreshCw,
    dropText: "Convert to another format",
    takesFiles: true,
    phase: 3,
  },
  {
    id: "screen-record",
    title: "Screen record",
    description: "Record your screen, a window or an area.",
    icon: MonitorDot,
    dropText: "Screen recording does not take files",
    takesFiles: false,
    phase: 14,
  },
];

export function quickToolById(id: string | null): QuickTool | undefined {
  return quickTools.find((tool) => tool.id === id);
}
