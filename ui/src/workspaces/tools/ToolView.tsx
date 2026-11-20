import { Redo2, Undo2 } from "lucide-react";

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import { IconButton } from "../../components/Button";
import { fileName } from "../../lib/routing";
import { isRunnable, type RunnableTool } from "../../lib/tools";
import { runCommand } from "../../state/commands";
import { useDocuments } from "../../state/documents";
import { quickTool } from "../quickTools";
import { Convert } from "./Convert";
import { ExportToSize } from "./ExportToSize";
import { GifMaker } from "./GifMaker";
import { GifTools } from "./GifTools";

const views: Record<RunnableTool, (props: { doc: DocumentInfo }) => React.JSX.Element> = {
  export_to_size: ExportToSize,
  video_to_gif: GifMaker,
  gif_tools: GifTools,
  convert: Convert,
};

/** a quick tool session: its header, then the tool itself */
export function ToolView({ doc }: { doc: DocumentInfo }) {
  const summary = useDocuments((s) => s.summaries[doc.id]);
  if (!isRunnable(doc.tool)) return null;
  const tool = quickTool(doc.tool);
  const View = views[doc.tool];
  const Icon = tool?.icon;
  const files =
    doc.inputs.length === 0
      ? "No files yet"
      : doc.inputs.length === 1
        ? fileName(doc.inputs[0] ?? "")
        : `${String(doc.inputs.length)} files`;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2.5">
        {Icon && (
          <span className="flex size-8 items-center justify-center rounded-lg bg-raised text-accent-text">
            <Icon className="size-4" strokeWidth={1.7} />
          </span>
        )}
        <div className="flex min-w-0 flex-1 flex-col">
          <h2 className="truncate text-base font-semibold tracking-tight">
            {tool?.title ?? doc.title}
          </h2>
          <span className="truncate text-xs text-fg-subtle">{files}</span>
        </div>
        <IconButton
          label={summary?.undoName ? `Undo ${summary.undoName}` : "Undo"}
          command="edit.undo"
          icon={<Undo2 className="size-4" />}
          disabled={!summary?.canUndo}
          onClick={() => void runCommand("edit.undo")}
        />
        <IconButton
          label={summary?.redoName ? `Redo ${summary.redoName}` : "Redo"}
          command="edit.redo"
          icon={<Redo2 className="size-4" />}
          disabled={!summary?.canRedo}
          onClick={() => void runCommand("edit.redo")}
        />
      </div>
      <View doc={doc} />
    </div>
  );
}
