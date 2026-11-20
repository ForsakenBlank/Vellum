import {
  ArrowLeftRight,
  Film,
  ImagePlay,
  Minimize2,
  MonitorDot,
  WandSparkles,
  type LucideIcon,
} from "lucide-react";

import type { FileKind } from "../bindings/FileKind";

export type QuickToolId =
  | "export_to_size"
  | "video_to_gif"
  | "gif_tools"
  | "remove_background"
  | "convert"
  | "screen_record";

/** the quick tool cards on the home screen. remove background arrives in phase 7 and screen
 * recording in phase 14. */
export interface QuickTool {
  id: QuickToolId;
  /** the app command that opens it */
  command: string;
  title: string;
  blurb: string;
  icon: LucideIcon;
  /** the file kinds it can take, empty for tools that make files rather than take them */
  accepts: FileKind[];
  /** what a drop does, eg "Make a GIF from clip.mp4" */
  verb: (what: string) => string;
  /** what it says when the dropped files do not fit, eg "Video to GIF needs a video" */
  needs: string;
  phase: number;
}

export const quickTools: QuickTool[] = [
  {
    id: "export_to_size",
    command: "tool.export_to_size",
    title: "Export to 10 MB",
    blurb: "Fit any clip, GIF, photo or song under a size limit.",
    icon: Minimize2,
    accepts: ["video", "gif", "image", "raw", "audio"],
    verb: (what) => `Shrink ${what} to 10 MB`,
    needs: "Export to 10 MB takes videos, GIFs, images and audio",
    phase: 3,
  },
  {
    id: "video_to_gif",
    command: "tool.video_to_gif",
    title: "Video to GIF",
    blurb: "Pick a moment, get a small, sharp GIF.",
    icon: ImagePlay,
    accepts: ["video", "gif"],
    verb: (what) => `Make a GIF from ${what}`,
    needs: "Video to GIF needs a video",
    phase: 3,
  },
  {
    id: "gif_tools",
    command: "tool.gif_tools",
    title: "GIF tools",
    blurb: "Edit frames, make it smaller, split it or turn it into a video.",
    icon: Film,
    accepts: ["gif"],
    verb: (what) => `Open ${what} in GIF tools`,
    needs: "GIF tools needs a GIF",
    phase: 3,
  },
  {
    id: "remove_background",
    command: "tool.remove_background",
    title: "Remove background",
    blurb: "Cut the subject out of a photo, on your machine.",
    icon: WandSparkles,
    accepts: ["image", "raw"],
    verb: (what) => `Remove the background from ${what}`,
    needs: "Remove background needs a photo",
    phase: 7,
  },
  {
    id: "convert",
    command: "tool.convert",
    title: "Convert",
    blurb: "Any format to any format, batches included.",
    icon: ArrowLeftRight,
    accepts: ["image", "raw", "gif", "video", "audio", "vector", "pdf"],
    verb: (what) => `Convert ${what}`,
    needs: "Convert cannot read that yet",
    phase: 3,
  },
  {
    id: "screen_record",
    command: "tool.screen_record",
    title: "Screen record",
    blurb: "Record the screen, a window or an area.",
    icon: MonitorDot,
    accepts: [],
    verb: () => "Start a screen recording",
    needs: "Screen record makes new videos, it does not take files",
    phase: 14,
  },
];

export function quickTool(id: string): QuickTool | undefined {
  return quickTools.find((tool) => tool.id === id);
}

export function quickToolForCommand(kind: string): QuickTool | undefined {
  return quickTools.find((tool) => tool.command === kind);
}
