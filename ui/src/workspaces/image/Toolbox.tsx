import {
  ArrowLeftRight,
  Brush,
  Eraser,
  Hand,
  Lasso,
  Move,
  PaintBucket,
  Pencil,
  Pipette,
  Circle,
  Square,
  SquareDashedMousePointer,
  Wand2,
  ZoomIn,
  Blend,
  Magnet,
  Spline,
  VenetianMask,
  type LucideIcon,
} from "lucide-react";

import { IconButton } from "../../components/Button";
import { Tooltip } from "../../components/Tooltip";
import { toHex } from "./geometry";
import { type ToolId, type ToolState, useImageTools } from "./store";

interface Entry {
  tool: ToolId;
  command: string;
  label: string;
  icon: (t: ToolState) => LucideIcon;
}

const entries: Entry[] = [
  { tool: "move", command: "image.tool.move", label: "Move tool", icon: () => Move },
  {
    tool: "marquee",
    command: "image.tool.marquee",
    label: "Marquee: drag to select. Shift adds, Alt takes away",
    icon: (t) => (t.marquee === "ellipse" ? Circle : Square),
  },
  {
    tool: "lasso",
    command: "image.tool.lasso",
    label: "Lasso: draw around something to select it",
    icon: (t) => (t.lasso === "magnetic" ? Magnet : t.lasso === "polygon" ? Spline : Lasso),
  },
  {
    tool: "wand",
    command: "image.tool.wand",
    label: "Magic wand and quick selection",
    icon: (t) => (t.wand === "quick" ? SquareDashedMousePointer : Wand2),
  },
  {
    tool: "eyedropper",
    command: "image.tool.eyedropper",
    label: "Eyedropper: pick a colour. Alt picks the background colour",
    icon: () => Pipette,
  },
  {
    tool: "brush",
    command: "image.tool.brush",
    label: "Brush and pencil",
    icon: (t) => (t.brushKind === "pencil" ? Pencil : Brush),
  },
  { tool: "eraser", command: "image.tool.eraser", label: "Eraser", icon: () => Eraser },
  {
    tool: "gradient",
    command: "image.tool.gradient",
    label: "Gradient and paint bucket",
    icon: (t) => (t.fill === "bucket" ? PaintBucket : Blend),
  },
  {
    tool: "hand",
    command: "image.tool.hand",
    label: "Hand: drag to move around. Hold Space with any tool",
    icon: () => Hand,
  },
  {
    tool: "zoom",
    command: "image.tool.zoom",
    label: "Zoom: click to zoom in, Alt click to zoom out",
    icon: () => ZoomIn,
  },
];

export function Toolbox({
  quickMask,
  onQuickMask,
}: {
  quickMask: boolean;
  onQuickMask: () => void;
}) {
  const t = useImageTools();
  return (
    <nav
      aria-label="Tools"
      className="flex w-11 shrink-0 flex-col items-center gap-0.5 border-r border-line bg-panel py-2"
    >
      {entries.map((e) => {
        const Icon = e.icon(t);
        return (
          <IconButton
            key={e.tool}
            label={e.label}
            command={e.command}
            side="right"
            active={t.tool === e.tool}
            icon={<Icon className="size-4" />}
            onClick={() => {
              t.set({ tool: e.tool });
            }}
          />
        );
      })}
      <span className="flex-1" />
      <div className="relative mb-1 size-9">
        <Tooltip text="Background colour" side="right">
          <span
            className="absolute right-0 bottom-0 size-6 rounded-sm border border-line-strong shadow-1"
            style={{ background: toHex(t.background) }}
          />
        </Tooltip>
        <Tooltip text="Foreground colour" side="right">
          <span
            className="absolute top-0 left-0 size-6 rounded-sm border border-line-strong shadow-1"
            style={{ background: toHex(t.foreground) }}
          />
        </Tooltip>
      </div>
      <IconButton
        size="sm"
        side="right"
        label="Swap colours"
        command="image.colors.swap"
        icon={<ArrowLeftRight className="size-3.5" />}
        onClick={() => {
          t.set({ foreground: t.background, background: t.foreground });
        }}
      />
      <IconButton
        side="right"
        label={quickMask ? "Leave quick mask" : "Quick mask: paint the selection"}
        command="image.quick_mask"
        active={quickMask}
        icon={<VenetianMask className="size-4" />}
        onClick={onQuickMask}
      />
    </nav>
  );
}
