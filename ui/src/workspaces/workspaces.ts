import {
  AudioWaveform,
  Clapperboard,
  Image,
  Palette,
  PenTool,
  Sparkles,
  Zap,
  type LucideIcon,
} from "lucide-react";

import type { Workspace } from "../bindings/Workspace";

/** the workspaces from PLAN.md section 4, with the phase that builds each one */
export interface WorkspaceInfo {
  id: Workspace;
  label: string;
  /** the app command that switches to it */
  command: string;
  phase: number;
  icon: LucideIcon;
  /** what the empty workspace says it is for */
  blurb: string;
}

export const workspaces: WorkspaceInfo[] = [
  {
    id: "quick-tools",
    label: "Quick tools",
    command: "workspace.quick_tools",
    phase: 3,
    icon: Zap,
    blurb: "Shrink, convert and turn clips into GIFs in one click.",
  },
  {
    id: "image",
    label: "Image",
    command: "workspace.image",
    phase: 5,
    icon: Image,
    blurb: "Layers, selections, brushes and filters.",
  },
  {
    id: "video",
    label: "Video",
    command: "workspace.video",
    phase: 8,
    icon: Clapperboard,
    blurb: "Cut, trim and arrange clips on a timeline.",
  },
  {
    id: "color",
    label: "Colour",
    command: "workspace.color",
    phase: 10,
    icon: Palette,
    blurb: "Grade footage with wheels, curves and scopes.",
  },
  {
    id: "audio",
    label: "Audio",
    command: "workspace.audio",
    phase: 11,
    icon: AudioWaveform,
    blurb: "Edit waveforms, clean up noise and mix.",
  },
  {
    id: "motion",
    label: "Motion",
    command: "workspace.motion",
    phase: 12,
    icon: Sparkles,
    blurb: "Titles, keyframes and compositing.",
  },
  {
    id: "vector",
    label: "Vector",
    command: "workspace.vector",
    phase: 15,
    icon: PenTool,
    blurb: "Shapes, paths, SVG and PDF.",
  },
];

export function workspaceInfo(id: Workspace): WorkspaceInfo {
  // every workspace id from rust is in the list, checked by a test
  return workspaces.find((w) => w.id === id) ?? (workspaces[0] as WorkspaceInfo);
}

export function workspaceForCommand(kind: string): WorkspaceInfo | undefined {
  return workspaces.find((w) => w.command === kind);
}
