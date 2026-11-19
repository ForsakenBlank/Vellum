import { Copy, Info } from "lucide-react";

import { IconButton } from "../../components/Button";
import { EmptyState } from "../../components/EmptyState";
import { formatBytes } from "../../lib/format";
import { fileName, kindOf } from "../../lib/routing";
import { activeDocument, useDocuments } from "../../state/documents";
import { useRecent } from "../../state/recent";
import { useUi } from "../../state/ui";
import { quickTool } from "../../workspaces/quickTools";
import { workspaceInfo } from "../../workspaces/workspaces";

/** details about the active document */
export function InfoPanel() {
  const doc = useDocuments(activeDocument);
  const summary = useDocuments((s) => (s.activeId ? s.summaries[s.activeId] : undefined));
  const recent = useRecent((s) => s.files);

  if (!doc) {
    return (
      <EmptyState icon={Info} title="No document open" compact>
        Open a file to see its type, location and size here.
      </EmptyState>
    );
  }

  const size = recent?.find((f) => f.path === doc.path)?.sizeBytes ?? null;
  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const rows: [string, string][] = [
    ["Workspace", workspaceInfo(doc.workspace).label],
    ["Type", doc.path ? (kindOf(doc.path)?.label ?? "Unknown") : tool ? "Quick tool" : "New"],
  ];
  if (size !== null) rows.push(["Size", formatBytes(size)]);
  if (tool) rows.push(["Tool", tool.title]);
  if (doc.inputs.length > 0) rows.push(["Files", doc.inputs.map(fileName).join(", ")]);
  rows.push(["Changes", doc.dirty ? "Not saved yet" : "Saved"]);
  if (summary?.undoName) rows.push(["Last step", summary.undoName]);

  const copyPath = (path: string) => {
    navigator.clipboard
      .writeText(path)
      .then(() => {
        useUi.getState().toast({ level: "success", title: "Copied the path" });
      })
      .catch(() => {
        useUi.getState().toast({ level: "error", title: "Could not copy the path" });
      });
  };

  return (
    <section aria-label="Info" className="flex flex-col gap-3 overflow-y-auto p-3">
      <div className="flex flex-col gap-0.5">
        <span className="truncate text-sm font-medium" title={doc.title}>
          {doc.title}
        </span>
        {doc.path && (
          <span className="flex items-center gap-1 text-xs text-fg-subtle">
            <span className="min-w-0 flex-1 truncate" title={doc.path}>
              {doc.path}
            </span>
            <IconButton
              size="sm"
              label="Copy path"
              icon={<Copy className="size-3" />}
              onClick={() => {
                if (doc.path) copyPath(doc.path);
              }}
            />
          </span>
        )}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-fg-subtle">{label}</dt>
            <dd className="min-w-0 truncate text-fg-muted tabular" title={value}>
              {value}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
