// works out what dropping files will do, before and after they land. the file type table
// comes from rust (crates/vellum-core/src/files.rs) through a generated json file, so the
// window and the cli always agree on where a file goes.

import type { FileKind } from "../bindings/FileKind";
import type { Workspace } from "../bindings/Workspace";
import table from "../bindings/fileKinds.json";
import { quickTool, type QuickToolId } from "../workspaces/quickTools";
import { workspaceInfo } from "../workspaces/workspaces";

export interface FileKindRow {
  kind: FileKind;
  label: string;
  workspace: Workspace;
  extensions: string[];
}

export const fileKinds = table as FileKindRow[];

export function extensionOf(path: string): string {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export function kindOf(path: string): FileKindRow | undefined {
  const ext = extensionOf(path);
  return ext ? fileKinds.find((row) => row.extensions.includes(ext)) : undefined;
}

/** where the files are dropped: anywhere on the window, or on one quick tool card */
export type DropTarget = { type: "window" } | { type: "tool"; tool: QuickToolId };

export type DropPlan =
  | {
      action: "open";
      files: string[];
      /** the workspace the first file opens in, which becomes the active tab */
      workspace: Workspace;
      title: string;
      detail: string;
    }
  | { action: "tool"; tool: QuickToolId; files: string[]; title: string; detail: string }
  | { action: "reject"; title: string; detail: string };

function describe(files: string[]): string {
  const [first] = files;
  return files.length === 1 && first ? fileName(first) : `${files.length} files`;
}

function skippedNote(skipped: string[]): string {
  if (skipped.length === 0) return "";
  if (skipped.length === 1 && skipped[0]) return `${fileName(skipped[0])} will be skipped.`;
  return `${skipped.length} files will be skipped.`;
}

function cannotOpen(files: string[]): string {
  const exts = [...new Set(files.map(extensionOf))];
  if (exts.length === 1 && exts[0]) return `Vellum cannot open .${exts[0]} files yet`;
  return "Vellum cannot open these files yet";
}

export function planDrop(files: string[], target: DropTarget): DropPlan {
  if (files.length === 0) {
    return { action: "reject", title: "Nothing to open", detail: "Only files can be dropped." };
  }

  if (target.type === "tool") {
    const tool = quickTool(target.tool);
    if (!tool) return { action: "reject", title: "Unknown tool", detail: "" };
    if (tool.accepts.length === 0) {
      return { action: "reject", title: tool.needs, detail: "Drop anywhere else to open it." };
    }
    const fits = files.filter((f) => {
      const row = kindOf(f);
      return row !== undefined && tool.accepts.includes(row.kind);
    });
    if (fits.length === 0) {
      return { action: "reject", title: tool.needs, detail: "Drop anywhere else to open it." };
    }
    const skipped = files.filter((f) => !fits.includes(f));
    return {
      action: "tool",
      tool: tool.id,
      files: fits,
      title: tool.verb(describe(fits)),
      detail: skippedNote(skipped) || `Opens in ${tool.title}.`,
    };
  }

  const known = files.filter((f) => kindOf(f));
  const skipped = files.filter((f) => !kindOf(f));
  const [first] = known;
  const firstRow = first ? kindOf(first) : undefined;
  if (!first || !firstRow) {
    return {
      action: "reject",
      title: cannotOpen(files),
      detail: "Try a common image, video or audio format.",
    };
  }

  const spaces = [...new Set(known.map((f) => kindOf(f)?.workspace))].filter(
    (w): w is Workspace => w !== undefined,
  );
  const where =
    spaces.length === 1
      ? `in ${workspaceInfo(firstRow.workspace).label}`
      : `in ${spaces.map((w) => workspaceInfo(w).label).join(", ")}`;
  const detail =
    skippedNote(skipped) ||
    (known.length === 1 ? `A ${firstRow.label} file.` : "Each file opens in its own tab.");
  return {
    action: "open",
    files: known,
    workspace: firstRow.workspace,
    title: `Open ${describe(known)} ${where}`,
    detail,
  };
}
