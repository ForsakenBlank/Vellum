import {
  AudioLines,
  File,
  FileText,
  Film,
  FolderClosed,
  Image,
  PenTool,
  type LucideIcon,
} from "lucide-react";

import type { MediaKind } from "../bindings/MediaKind";

const icons: Record<MediaKind, LucideIcon> = {
  image: Image,
  video: Film,
  audio: AudioLines,
  vector: PenTool,
  document: FileText,
  project: FolderClosed,
  unknown: File,
};

export function mediaIcon(kind: MediaKind): LucideIcon {
  return icons[kind];
}
