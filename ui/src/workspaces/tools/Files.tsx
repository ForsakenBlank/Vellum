// the files column every quick tool shares: one row per file with its thumbnail, live
// progress and, when it is done, the result with its size, what changed, and the buttons
// to open it, show it in its folder or copy it for pasting into a chat.
//
// the column is also a drop zone: files dropped on it join this session instead of opening
// a new one.

import { convertFileSrc } from "@tauri-apps/api/core";
import {
  Check,
  ClipboardCopy,
  ExternalLink,
  FileWarning,
  FolderOpen,
  Plus,
  RotateCcw,
  SplitSquareHorizontal,
  X,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { JobSnapshot } from "../../bindings/JobSnapshot";
import type { ToolResult } from "../../bindings/ToolResult";
import { Button, IconButton } from "../../components/Button";
import { ProgressBar } from "../../components/ProgressBar";
import { Skeleton } from "../../components/Skeleton";
import { formatBytes, formatPercent } from "../../lib/format";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { fileName, kindOf } from "../../lib/routing";
import { latestPerFile, savings, toolResult } from "../../lib/tools";
import { assetFile, useAsset } from "../../state/media";
import { useUi } from "../../state/ui";
import { workspaceInfo } from "../workspaces";
import { fail } from "./session";

const busy = ["queued", "running", "pausing", "paused", "cancelling", "waitingToRetry"];

export function FilesColumn({
  doc,
  jobs,
  onAdd,
  onRemove,
  runLabel,
  runTooltip,
  onRun,
  canRun,
  footer,
}: {
  doc: DocumentInfo;
  jobs: JobSnapshot[];
  onAdd: () => void;
  onRemove: (path: string) => void;
  runLabel: string;
  runTooltip: string;
  onRun: () => void;
  canRun: boolean;
  /** shown above the run button, eg a size estimate */
  footer?: ReactNode;
}) {
  const latest = latestPerFile(jobs);
  // a slideshow is one job for all the pictures, filed under the first one
  const files = doc.inputs;
  const working = jobs.some((job) => busy.includes(job.state));
  const drop = useUi((s) => s.drop);
  const hovered = drop?.action === "add" && drop.document === doc.id;

  return (
    <aside
      aria-label="Files"
      data-drop-session={doc.id}
      data-drop-session-tool={doc.tool ?? undefined}
      className={`flex min-h-0 flex-col border-t border-line bg-panel transition-shadow duration-(--motion-fast) lg:border-t-0 lg:border-l ${
        hovered ? "shadow-[inset_0_0_0_2px_var(--accent)]" : ""
      }`}
    >
      <div className="flex items-center justify-between gap-2 px-4 py-2.5">
        <h3 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">
          {files.length === 1 ? "1 file" : `${String(files.length)} files`}
        </h3>
        <Button
          size="sm"
          variant="ghost"
          tooltip="Add more files to this batch"
          icon={<Plus className="size-3.5" />}
          disabled={!inDesktopApp}
          onClick={onAdd}
        >
          Add files
        </Button>
      </div>
      <ul className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-3 pb-3">
        {files.length === 0 && (
          <li className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-line-strong px-4 py-8 text-center text-xs text-fg-muted">
            Drop files here, or use Add files.
          </li>
        )}
        {files.map((path) => (
          <FileRow
            key={path}
            path={path}
            job={latest.get(path) ?? null}
            onRemove={() => {
              onRemove(path);
            }}
            locked={busy.includes(latest.get(path)?.state ?? "")}
          />
        ))}
      </ul>
      <div className="flex flex-col gap-2 border-t border-line p-3">
        {footer}
        <Button
          size="lg"
          variant="primary"
          tooltip={runTooltip}
          disabled={!canRun || files.length === 0 || !inDesktopApp}
          onClick={onRun}
          className="w-full"
        >
          {working ? "Working..." : runLabel}
        </Button>
      </div>
    </aside>
  );
}

function FileRow({
  path,
  job,
  onRemove,
  locked,
}: {
  path: string;
  job: JobSnapshot | null;
  onRemove: () => void;
  locked: boolean;
}) {
  const result = job?.state === "completed" ? toolResult(job) : null;
  return (
    <li className="flex flex-col gap-2 rounded-lg border border-line bg-raised p-2.5">
      <div className="flex items-center gap-2.5">
        <Thumb path={path} />
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm" title={path}>
            {fileName(path)}
          </span>
          <Status file={path} job={job} result={result} />
        </div>
        <IconButton
          size="sm"
          label={
            locked ? "Wait for this file to finish before removing it" : "Remove from the batch"
          }
          icon={<X className="size-3.5" />}
          disabled={locked}
          onClick={onRemove}
        />
      </div>
      {job && busy.includes(job.state) && (
        <div className="flex flex-col gap-1">
          <ProgressBar value={job.progress} label={`Progress of ${fileName(path)}`} />
          <span className="text-2xs text-fg-subtle tabular">
            {job.message ?? (job.state === "queued" ? "Waiting for a free slot" : "Working")}
            {job.progress !== null ? ` · ${formatPercent(job.progress)}` : ""}
          </span>
        </div>
      )}
      {job?.state === "failed" && (
        <div className="flex items-start gap-2 rounded-md bg-danger/10 px-2 py-1.5 text-xs text-danger">
          <FileWarning className="mt-px size-3.5 shrink-0" />
          <span className="flex-1">{job.error ?? "It failed."}</span>
          <IconButton
            size="sm"
            label="Try again"
            icon={<RotateCcw className="size-3.5" />}
            onClick={() => {
              ipc.retryJob(job.id).catch(fail("It could not be retried"));
            }}
          />
        </div>
      )}
      {result && <ResultCard result={result} />}
    </li>
  );
}

function Status({
  file,
  job,
  result,
}: {
  file: string;
  job: JobSnapshot | null;
  result: ToolResult | null;
}) {
  if (result) {
    if (result.alreadyFits) {
      return <span className="text-2xs text-success">Already small enough</span>;
    }
    return (
      <span className="flex items-center gap-1 text-2xs text-success tabular">
        <Check className="size-3" />
        {result.folder ? "Done" : result.bytes !== null ? formatBytes(result.bytes) : "Done"}
        {result.bytes !== null && result.sourceBytes !== null && !result.folder
          ? ` · ${savings(result.sourceBytes, result.bytes) ?? ""}`
          : ""}
      </span>
    );
  }
  if (job?.state === "cancelled") return <span className="text-2xs text-fg-subtle">Cancelled</span>;
  if (job && busy.includes(job.state)) return null;
  return <span className="text-2xs text-fg-subtle">{kindOf(file)?.label ?? "Ready"}</span>;
}

function Thumb({ path }: { path: string }) {
  const asset = useAsset("thumbnail", inDesktopApp ? path : null);
  const file = assetFile(asset);
  const row = kindOf(path);
  const Icon = row ? workspaceInfo(row.workspace).icon : FolderOpen;
  return (
    <div className="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-md border border-line bg-elevated text-fg-subtle">
      {file ? (
        <img
          src={convertFileSrc(file)}
          alt=""
          draggable={false}
          className="checker size-full object-cover"
        />
      ) : asset?.state === "pending" ? (
        <Skeleton className="size-full rounded-none" />
      ) : (
        <Icon className="size-4" strokeWidth={1.5} />
      )}
    </div>
  );
}

function ResultCard({ result }: { result: ToolResult }) {
  const [compare, setCompare] = useState(false);
  const onlyImages = !result.folder && !result.alreadyFits;
  const copy = () => {
    ipc
      .copyFiles([result.path])
      .then(() => {
        useUi.getState().toast({
          level: "success",
          title: "Copied",
          body: `Paste ${fileName(result.path)} into a chat or a folder.`,
        });
      })
      .catch(fail("It could not be copied"));
  };
  return (
    <div className="flex flex-col gap-2">
      {result.targetBytes !== null && result.bytes !== null && !result.alreadyFits && (
        <p className="text-2xs text-fg-subtle tabular">
          {formatBytes(result.bytes)} of the {formatBytes(result.targetBytes)} allowed
        </p>
      )}
      {result.changes.length > 0 && (
        <ul className="flex flex-col gap-0.5 text-2xs text-fg-muted">
          {result.changes.map((change) => (
            <li key={change}>{change}</li>
          ))}
        </ul>
      )}
      {result.notes.map((note) => (
        <p key={note} className="rounded-md bg-accent-soft px-2 py-1.5 text-2xs text-accent-text">
          {note}
        </p>
      ))}
      <div className="flex flex-wrap gap-1.5">
        <Button
          size="sm"
          tooltip={result.folder ? "Open the folder" : "Open with your default app"}
          icon={<ExternalLink className="size-3.5" />}
          onClick={() => {
            ipc.openResult(result.path, false).catch(fail("It could not be opened"));
          }}
        >
          Open
        </Button>
        <Button
          size="sm"
          tooltip="Show it in its folder"
          icon={<FolderOpen className="size-3.5" />}
          onClick={() => {
            ipc.openResult(result.path, true).catch(fail("The folder could not be shown"));
          }}
        >
          Show in folder
        </Button>
        {!result.folder && (
          <Button
            size="sm"
            tooltip="Copy the file, then paste it straight into Discord or any chat"
            icon={<ClipboardCopy className="size-3.5" />}
            onClick={copy}
          >
            Copy as file
          </Button>
        )}
        {onlyImages && (
          <Button
            size="sm"
            variant={compare ? "primary" : "secondary"}
            tooltip="Compare the original and the result side by side"
            icon={<SplitSquareHorizontal className="size-3.5" />}
            onClick={() => {
              setCompare(!compare);
            }}
          >
            Before and after
          </Button>
        )}
      </div>
      {compare && <Compare before={result.source} after={result.path} />}
    </div>
  );
}

/** a slider over the original and the result. still pictures show at full size, videos
 * and everything else compare their thumbnails. */
export function Compare({ before, after }: { before: string; after: string }) {
  const [split, setSplit] = useState(0.5);
  const still =
    (kindOf(before)?.kind === "image" || kindOf(before)?.kind === "raw") &&
    kindOf(after)?.kind === "image";
  const [ready, setReady] = useState(!still || !inDesktopApp);
  const beforeThumb = assetFile(useAsset("thumbnail", still ? null : before));
  const afterThumb = assetFile(useAsset("thumbnail", still ? null : after));

  // full size pictures need the window to be allowed to load them
  useEffect(() => {
    if (!still || !inDesktopApp) return;
    Promise.all([ipc.previewFile(before), ipc.previewFile(after)])
      .then(() => {
        setReady(true);
      })
      .catch(fail("The comparison could not be shown"));
  }, [still, before, after]);
  const left = still ? before : beforeThumb;
  const right = still ? after : afterThumb;
  if (!left || !right || !ready) {
    return <Skeleton className="aspect-video w-full rounded-md" />;
  }
  return (
    <div className="flex flex-col gap-1.5">
      <div className="checker relative aspect-video w-full overflow-hidden rounded-md border border-line select-none">
        <img
          src={convertFileSrc(right)}
          alt="After"
          className="absolute inset-0 size-full object-contain"
        />
        <div
          className="absolute inset-0 overflow-hidden"
          style={{ clipPath: `inset(0 ${String((1 - split) * 100)}% 0 0)` }}
        >
          <img
            src={convertFileSrc(left)}
            alt="Before"
            className="absolute inset-0 size-full object-contain"
          />
        </div>
        <div
          className="pointer-events-none absolute inset-y-0 w-px bg-accent"
          style={{ left: `${String(split * 100)}%` }}
        />
        <span className="absolute top-1.5 left-1.5 rounded-sm bg-canvas/80 px-1.5 text-2xs">
          Before
        </span>
        <span className="absolute top-1.5 right-1.5 rounded-sm bg-canvas/80 px-1.5 text-2xs">
          After
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={1}
        step={0.01}
        value={split}
        aria-label="Move the line between before and after"
        onChange={(event) => {
          setSplit(Number(event.target.value));
        }}
        className="accent-(--accent)"
      />
    </div>
  );
}
