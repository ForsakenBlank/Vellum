import { clsx } from "clsx";
import { FileUp, FolderOpen } from "lucide-react";
import { useState } from "react";

import type { OpenDocument } from "../bindings/OpenDocument";
import { useShortcut } from "../commands/registry";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { MediaIcon } from "../components/MediaIcon";
import { Dock } from "../dock/Dock";
import { perform, useApp } from "../stores/app";
import { pickAndOpen } from "../stores/session";
import type { Workspace } from "../workspaces/types";
import { useDrop } from "./drop";
import { quickToolById } from "./quickTools";

/** a workspace with its docked panels around whatever it is showing */
export function WorkspaceView({
  workspace,
  document,
}: {
  workspace: Workspace;
  document: OpenDocument | null;
}) {
  const Editor = workspace.component;
  return (
    <Dock workspace={workspace}>
      {document && Editor ? (
        <Editor document={document} />
      ) : document ? (
        <DocumentPlaceholder workspace={workspace} document={document} />
      ) : (
        <WorkspaceEmpty workspace={workspace} />
      )}
    </Dock>
  );
}

/** a workspace with nothing open: what it is for and how to start */
function WorkspaceEmpty({ workspace }: { workspace: Workspace }) {
  const ready = useApp((state) => state.status === "ready");
  const openKey = useShortcut("file.open");
  const key = `workspace:${workspace.id}`;
  const hovered = useDrop((state) => state.targetKey === key);
  return (
    <div data-drop={key} className="h-full bg-sunken p-6">
      <div
        className={clsx(
          "h-full rounded-2xl border-2 border-dashed transition-colors duration-(--v-fast)",
          hovered ? "border-accent bg-accent-soft" : "border-line",
        )}
      >
        <EmptyState
          icon={workspace.icon}
          title={`Nothing open in ${workspace.title}`}
          body={`Drop ${workspace.accepts} here, or open a file. ${workspace.summary}`}
        >
          <Button
            variant="primary"
            tip="Choose files to open here"
            shortcut={openKey}
            disabled={!ready}
            onClick={() => void pickAndOpen({ type: "workspace", id: workspace.id })}
          >
            <FolderOpen className="size-4" aria-hidden />
            Open file
          </Button>
        </EmptyState>
      </div>
    </div>
  );
}

/**
 * stands in for an editor until its phase is built. it shows the file so
 * opening and routing can be checked by eye, and says when editing arrives
 */
function DocumentPlaceholder({
  workspace,
  document,
}: {
  workspace: Workspace;
  document: OpenDocument;
}) {
  const previewUrl = useApp((state) => state.previewUrl);
  const [broken, setBroken] = useState(false);
  const tool = quickToolById(document.tool);
  const showImage =
    previewUrl && document.path && !broken && ["image", "vector"].includes(document.kind);
  const phase = tool?.phase ?? workspace.phase;
  const path = document.path;

  return (
    <div className="flex h-full flex-col items-center justify-center gap-5 bg-sunken p-8">
      {showImage ? (
        <img
          src={previewUrl(path ?? "")}
          alt={document.title}
          onError={() => {
            setBroken(true);
          }}
          className="max-h-[55%] max-w-[80%] rounded-md object-contain shadow-modal [background:repeating-conic-gradient(var(--v-raised)_0_25%,var(--v-surface)_0_50%)_0_0/16px_16px]"
        />
      ) : (
        <div className="flex size-20 items-center justify-center rounded-2xl border border-line bg-surface text-ink-soft">
          {tool ? (
            <tool.icon className="size-8" strokeWidth={1.5} aria-hidden />
          ) : (
            <MediaIcon kind={document.kind} className="size-8" strokeWidth={1.5} />
          )}
        </div>
      )}
      <div className="max-w-md text-center">
        <h1 className="text-lg font-semibold">
          {tool ? `${tool.title}: ${document.title}` : document.title}
        </h1>
        <p className="mt-1 text-sm text-ink-soft select-text">{path}</p>
        <p className="mt-3 text-sm text-ink-soft">
          {tool
            ? `${tool.title} arrives in phase ${phase}.`
            : `Editing in ${workspace.title} arrives in phase ${phase}.`}
        </p>
      </div>
      {path && (
        <div className="flex gap-2">
          <Button
            tip="Show this file in your file manager"
            onClick={() => void perform((api) => api.files.reveal(path))}
          >
            <FolderOpen className="size-4" aria-hidden />
            Show in folder
          </Button>
          <Button
            tip="Open another file in this workspace"
            onClick={() => void pickAndOpen({ type: "workspace", id: workspace.id })}
          >
            <FileUp className="size-4" aria-hidden />
            Open another
          </Button>
        </div>
      )}
    </div>
  );
}
