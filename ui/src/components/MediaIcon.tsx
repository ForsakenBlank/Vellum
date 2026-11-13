import {
  AudioLines,
  File,
  FileText,
  Film,
  FolderClosed,
  Image,
  PenTool,
  type LucideProps,
} from "lucide-react";

import type { MediaKind } from "../bindings/MediaKind";

/** the icon for a kind of file */
export function MediaIcon({ kind, ...props }: { kind: MediaKind } & LucideProps) {
  switch (kind) {
    case "image":
      return <Image aria-hidden {...props} />;
    case "video":
      return <Film aria-hidden {...props} />;
    case "audio":
      return <AudioLines aria-hidden {...props} />;
    case "vector":
      return <PenTool aria-hidden {...props} />;
    case "document":
      return <FileText aria-hidden {...props} />;
    case "project":
      return <FolderClosed aria-hidden {...props} />;
    case "unknown":
      return <File aria-hidden {...props} />;
  }
}
