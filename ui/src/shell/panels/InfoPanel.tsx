import { ClipboardCopy, Copy, EyeOff, Info, ShieldAlert } from "lucide-react";
import { useEffect, useState } from "react";

import type { MediaInfo } from "../../bindings/MediaInfo";
import type { MetadataTag } from "../../bindings/MetadataTag";
import { Button, IconButton } from "../../components/Button";
import { EmptyState } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { describeMedia, mediaInfoText, type InfoRow } from "../../lib/mediaInfo";
import { fileName, kindOf } from "../../lib/routing";
import { activeDocument, useDocuments } from "../../state/documents";
import { useUi } from "../../state/ui";
import { quickTool } from "../../workspaces/quickTools";
import { workspaceInfo } from "../../workspaces/workspaces";

type Probe =
  { state: "loading" } | { state: "ready"; info: MediaInfo } | { state: "failed"; error: string };

/** what the media core says about a file, read again when the path changes */
function useMediaInfo(path: string | null): Probe | null {
  const [probe, setProbe] = useState<{ path: string; result: Probe } | null>(null);
  useEffect(() => {
    if (!path || !inDesktopApp) return;
    let live = true;
    ipc
      .probeMedia(path)
      .then((info) => {
        if (live) setProbe({ path, result: { state: "ready", info } });
      })
      .catch((err: unknown) => {
        if (live) setProbe({ path, result: { state: "failed", error: String(err) } });
      });
    return () => {
      live = false;
    };
  }, [path]);
  if (!path || !inDesktopApp) return null;
  return probe?.path === path ? probe.result : { state: "loading" };
}

function copyText(text: string, what: string) {
  navigator.clipboard
    .writeText(text)
    .then(() => {
      useUi.getState().toast({ level: "success", title: `Copied the ${what}` });
    })
    .catch(() => {
      useUi.getState().toast({ level: "error", title: `Could not copy the ${what}` });
    });
}

/** details about the active document and the file behind it */
export function InfoPanel() {
  const doc = useDocuments(activeDocument);
  const summary = useDocuments((s) => (s.activeId ? s.summaries[s.activeId] : undefined));
  const probe = useMediaInfo(doc?.path ?? null);

  if (!doc) {
    return (
      <EmptyState icon={Info} title="No document open" compact>
        Open a file to see its format, size, codecs and hidden metadata here.
      </EmptyState>
    );
  }

  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const documentRows: InfoRow[] = [
    { label: "Workspace", value: workspaceInfo(doc.workspace).label },
    {
      label: "Type",
      value: doc.path ? (kindOf(doc.path)?.label ?? "Unknown") : tool ? "Quick tool" : "New",
    },
  ];
  if (tool) documentRows.push({ label: "Tool", value: tool.title });
  if (doc.inputs.length > 0) {
    documentRows.push({ label: "Files", value: doc.inputs.map(fileName).join(", ") });
  }
  documentRows.push({ label: "Changes", value: doc.dirty ? "Not saved yet" : "Saved" });
  if (summary?.undoName) documentRows.push({ label: "Last step", value: summary.undoName });

  return (
    <section aria-label="Info" className="flex flex-col gap-4 overflow-y-auto p-3">
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
                if (doc.path) copyText(doc.path, "path");
              }}
            />
          </span>
        )}
      </div>

      {probe && <MediaDetails probe={probe} />}
      <Rows title="Document" rows={documentRows} />
    </section>
  );
}

function MediaDetails({ probe }: { probe: Probe }) {
  if (probe.state === "loading") {
    return (
      <div aria-label="Reading the file" className="flex flex-col gap-2">
        <Skeleton className="h-3 w-24" />
        <Skeleton className="h-3 w-40" />
        <Skeleton className="h-3 w-32" />
      </div>
    );
  }
  if (probe.state === "failed") {
    return (
      <p role="alert" className="rounded-md bg-danger/10 px-2.5 py-2 text-xs text-danger">
        {probe.error}
      </p>
    );
  }
  const { info } = probe;
  return (
    <>
      <div className="flex gap-1.5">
        <Button
          size="sm"
          icon={<ClipboardCopy className="size-3.5" />}
          tooltip="Copy every detail below as text"
          onClick={() => {
            copyText(mediaInfoText(info), "file info");
          }}
        >
          Copy info
        </Button>
        <StripButton info={info} />
      </div>
      {describeMedia(info).map((section) => (
        <Rows key={section.title} title={section.title} rows={section.rows} />
      ))}
      {info.warnings.map((warning) => (
        <p key={warning} className="text-xs text-warning">
          {warning}
        </p>
      ))}
      <Metadata tags={info.tags} />
    </>
  );
}

function StripButton({ info }: { info: MediaInfo }) {
  const [busy, setBusy] = useState(false);
  return (
    <Button
      size="sm"
      icon={<EyeOff className="size-3.5" />}
      tooltip="Save a copy without location, camera and other hidden details. The original is not changed."
      disabled={busy}
      onClick={() => {
        setBusy(true);
        ipc
          .stripMetadata(info.path)
          .then(() => {
            useUi.getState().toast({
              level: "info",
              title: "Removing metadata",
              body: "A clean copy will appear next to the original.",
            });
          })
          .catch((err: unknown) => {
            useUi
              .getState()
              .toast({ level: "error", title: "Metadata could not be removed", body: String(err) });
          })
          .finally(() => {
            setBusy(false);
          });
      }}
    >
      Strip metadata
    </Button>
  );
}

function Rows({ title, rows }: { title: string; rows: InfoRow[] }) {
  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">{title}</h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
        {rows.map((row) => (
          <div key={row.label} className="contents">
            <dt className="text-fg-subtle">{row.label}</dt>
            <dd className="min-w-0 truncate text-fg-muted tabular select-text" title={row.value}>
              {row.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function Metadata({ tags }: { tags: MetadataTag[] }) {
  const [open, setOpen] = useState(false);
  if (tags.length === 0) {
    return (
      <div className="flex flex-col gap-1.5">
        <h3 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">Metadata</h3>
        <p className="text-xs text-fg-subtle">No hidden metadata.</p>
      </div>
    );
  }
  const private_ = tags.filter((t) => t.sensitive).length;
  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">Metadata</h3>
      <p className="flex items-center gap-1.5 text-xs text-fg-muted">
        {private_ > 0 && <ShieldAlert aria-hidden className="size-3.5 text-warning" />}
        {tags.length} {tags.length === 1 ? "tag" : "tags"}
        {private_ > 0 && `, ${String(private_)} may be private`}
      </p>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
        className="self-start text-xs text-accent hover:underline"
      >
        {open ? "Hide tags" : "Show tags"}
      </button>
      {open && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          {tags.map((tag, i) => (
            <div key={`${tag.group}:${tag.key}:${String(i)}`} className="contents">
              <dt
                className={`truncate ${tag.sensitive ? "text-warning" : "text-fg-subtle"}`}
                title={`${tag.group} / ${tag.key}`}
              >
                {tag.key}
              </dt>
              <dd className="min-w-0 truncate text-fg-muted select-text" title={tag.value}>
                {tag.value}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}
