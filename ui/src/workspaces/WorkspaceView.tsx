import { FolderOpen, Redo2, Save, Undo2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { DocumentInfo } from "../bindings/DocumentInfo";
import type { Workspace } from "../bindings/Workspace";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Skeleton } from "../components/Skeleton";
import { inDesktopApp, ipc } from "../lib/ipc";
import { fileName, kindOf } from "../lib/routing";
import { runCommand } from "../state/commands";
import { activeDocument, useDocuments } from "../state/documents";
import { useUi } from "../state/ui";
import { pickAndOpen } from "../shell/actions";
import { quickTool } from "./quickTools";
import { workspaceInfo } from "./workspaces";

/** the centre of the window: the active document, or an empty workspace */
export function WorkspaceView() {
  const doc = useDocuments(activeDocument);
  const activeId = useDocuments((s) => s.activeId);
  const empty = useDocuments((s) => s.emptyWorkspace);
  if (doc) return <DocumentView key={doc.id} doc={doc} />;
  // the tab was just opened and the list has not caught up yet
  if (activeId) return <Loading />;
  if (empty) return <EmptyWorkspace workspace={empty} />;
  return null;
}

function Loading() {
  return (
    <div className="flex h-full flex-col gap-4 p-8" aria-busy aria-label="Loading">
      <Skeleton className="h-7 w-64" />
      <Skeleton className="w-full flex-1 rounded-xl" />
    </div>
  );
}

function EmptyWorkspace({ workspace }: { workspace: Workspace }) {
  const info = workspaceInfo(workspace);
  return (
    <div className="grain flex h-full items-center justify-center">
      <EmptyState
        icon={info.icon}
        title={`${info.label} workspace`}
        actions={
          <Button
            variant="primary"
            tooltip="Pick files to open"
            command="file.open"
            icon={<FolderOpen className="size-4" />}
            disabled={!inDesktopApp}
            onClick={() => void pickAndOpen()}
          >
            Open files
          </Button>
        }
      >
        {info.blurb} Drop a file anywhere to open it here. The editor itself arrives in phase{" "}
        {info.phase}.
      </EmptyState>
    </div>
  );
}

interface PlaceholderState {
  title: string;
  notes: string;
}

function readState(value: unknown): PlaceholderState {
  const state = (typeof value === "object" && value !== null ? value : {}) as Record<
    string,
    unknown
  >;
  return {
    title: typeof state.title === "string" ? state.title : "",
    notes: typeof state.notes === "string" ? state.notes : "",
  };
}

/** how long typing pauses before the notes are sent, they merge into one undo step */
const NOTES_DEBOUNCE_MS = 300;

function DocumentView({ doc }: { doc: DocumentInfo }) {
  const summary = useDocuments((s) => s.summaries[doc.id]);
  const [state, setState] = useState<PlaceholderState | null>(null);
  const [notes, setNotes] = useState("");
  const typing = useRef(false);
  const timer = useRef<number | null>(null);
  const space = workspaceInfo(doc.workspace);
  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const Icon = tool?.icon ?? space.icon;

  // reload after every history step, so undo and redo show up here
  useEffect(() => {
    if (!inDesktopApp) return;
    let stale = false;
    ipc
      .getDocument(doc.id)
      .then((result) => {
        if (stale) return;
        const next = readState(result.state);
        setState(next);
        if (!typing.current) setNotes(next.notes);
      })
      .catch(() => {
        // the document closed while loading, the tab is about to go
      });
    return () => {
      stale = true;
    };
  }, [doc.id, summary]);

  const fail = (err: unknown) => {
    useUi.getState().toast({ level: "error", title: "The change was not made", body: String(err) });
  };

  const sendNotes = (text: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      ipc.documentCommand(doc.id, "document.edit_notes", { notes: text }).catch(fail);
    }, NOTES_DEBOUNCE_MS);
  };

  const finishTyping = () => {
    typing.current = false;
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
      void ipc
        .documentCommand(doc.id, "document.edit_notes", { notes })
        .then(() => ipc.sealDocument(doc.id))
        .catch(fail);
    } else {
      ipc.sealDocument(doc.id).catch(fail);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2.5">
        <span className="flex size-8 items-center justify-center rounded-lg bg-raised text-accent-text">
          <Icon className="size-4" strokeWidth={1.7} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col">
          <TitleField doc={doc} onFail={fail} />
          <span className="truncate text-xs text-fg-subtle">
            {tool
              ? `${tool.title} · arrives in phase ${tool.phase}`
              : `${space.label} workspace · arrives in phase ${space.phase}`}
            {doc.path ? ` · ${doc.path}` : ""}
          </span>
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
        <Button
          size="sm"
          tooltip={doc.dirty ? "Save the changes" : "Nothing to save"}
          command="file.save"
          icon={<Save className="size-3.5" />}
          disabled={!doc.dirty}
          onClick={() => void runCommand("file.save")}
        >
          Save
        </Button>
      </div>

      <div className="flex min-h-0 flex-1 gap-4 p-4">
        <Canvas doc={doc} />
        <aside className="flex w-72 shrink-0 flex-col gap-2">
          <label htmlFor={`notes-${doc.id}`} className="text-xs font-medium text-fg-muted">
            Notes
          </label>
          {state === null && inDesktopApp ? (
            <Skeleton className="flex-1 rounded-lg" />
          ) : (
            <textarea
              id={`notes-${doc.id}`}
              value={notes}
              placeholder="Write anything. Every pause in typing is one undo step, so you can try undo, redo, autosave and crash recovery here."
              onFocus={() => {
                typing.current = true;
              }}
              onChange={(event) => {
                setNotes(event.target.value);
                sendNotes(event.target.value);
              }}
              onBlur={finishTyping}
              className="min-h-0 flex-1 resize-none rounded-lg border border-line bg-raised p-3 text-sm leading-relaxed text-fg placeholder:text-fg-subtle focus:border-accent-line focus:outline-none"
            />
          )}
          <p className="text-2xs text-fg-subtle">
            Placeholder documents have no file format yet, so Save only marks them as saved.
          </p>
        </aside>
      </div>
    </div>
  );
}

/** the document name, renamed in place with a double click, as one undoable step */
function TitleField({ doc, onFail }: { doc: DocumentInfo; onFail: (err: unknown) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(doc.title);

  const commit = () => {
    setEditing(false);
    const title = draft.trim();
    if (title && title !== doc.title) {
      ipc.documentCommand(doc.id, "document.rename", { title }).catch(onFail);
    }
  };

  if (editing) {
    return (
      <input
        autoFocus
        aria-label="Document name"
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
        }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setEditing(false);
        }}
        className="-mx-1 rounded-sm bg-raised px-1 text-base font-semibold outline-none focus:shadow-(--ring)"
      />
    );
  }
  return (
    <button
      type="button"
      title="Double click to rename"
      onDoubleClick={() => {
        setDraft(doc.title);
        setEditing(true);
      }}
      className="flex items-center gap-2 truncate text-left text-base font-semibold tracking-tight"
    >
      <span className="truncate">{doc.title}</span>
      {doc.dirty && <span className="shrink-0 text-2xs font-normal text-fg-subtle">Edited</span>}
    </button>
  );
}

/** a stand in for the real editor, shaped like the workspace that will replace it */
function Canvas({ doc }: { doc: DocumentInfo }) {
  const space = workspaceInfo(doc.workspace);
  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const Icon = tool?.icon ?? space.icon;
  const files = doc.inputs.length > 0 ? doc.inputs : doc.path ? [doc.path] : [];
  const imageLike = doc.workspace === "image" || doc.workspace === "vector";

  return (
    <div
      className={`relative flex min-w-0 flex-1 flex-col items-center justify-center gap-4 overflow-hidden rounded-xl border border-line ${imageLike ? "checker" : "grain bg-canvas"}`}
    >
      {doc.workspace === "video" && <TimelineSketch />}
      {doc.workspace === "audio" && <WaveSketch />}
      <div className="flex flex-col items-center gap-3 rounded-xl bg-panel/85 px-6 py-5 text-center shadow-2 backdrop-blur">
        <Icon className="size-6 text-accent-text" strokeWidth={1.5} />
        <div className="flex flex-col gap-1">
          <p className="text-sm font-medium">
            {tool
              ? `${tool.title} is on its way`
              : `The ${space.label.toLowerCase()} editor is on its way`}
          </p>
          <p className="max-w-80 text-xs text-fg-muted">
            {tool
              ? tool.blurb
              : `${space.blurb} It replaces this placeholder in phase ${space.phase}.`}
          </p>
        </div>
        {files.length > 0 && (
          <ul className="flex max-w-80 flex-wrap justify-center gap-1.5">
            {files.map((file) => (
              <li
                key={file}
                title={file}
                className="max-w-52 truncate rounded-md border border-line bg-raised px-2 py-0.5 text-2xs text-fg-muted"
              >
                {fileName(file)}
                {kindOf(file) ? "" : " (not supported)"}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function TimelineSketch() {
  return (
    <div aria-hidden className="absolute inset-x-6 bottom-6 flex flex-col gap-1.5 opacity-40">
      {[0.62, 0.38, 0.8].map((width, i) => (
        <div
          key={i}
          className="h-5 rounded-sm bg-pressed"
          style={{ width: `${width * 100}%`, marginLeft: `${i * 6}%` }}
        />
      ))}
    </div>
  );
}

function WaveSketch() {
  const bars = Array.from(
    { length: 96 },
    (_, i) => 0.2 + Math.abs(Math.sin(i * 0.37) * Math.cos(i * 0.11)) * 0.8,
  );
  return (
    <div
      aria-hidden
      className="absolute inset-x-6 top-1/2 flex h-24 -translate-y-1/2 items-center gap-[2px] opacity-30"
    >
      {bars.map((h, i) => (
        <span
          key={i}
          className="flex-1 rounded-full bg-fg-muted"
          style={{ height: `${h * 100}%` }}
        />
      ))}
    </div>
  );
}
