import { convertFileSrc } from "@tauri-apps/api/core";
import { ClipboardCopy, Copy, Eraser, Info, MapPin } from "lucide-react";
import { useEffect, useState } from "react";

import type { MediaInfo } from "../../bindings/MediaInfo";
import type { WaveformOverview } from "../../bindings/WaveformOverview";
import { Button, IconButton } from "../../components/Button";
import { EmptyState } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { formatBytes } from "../../lib/format";
import { inDesktopApp, ipc } from "../../lib/ipc";
import {
  infoSections,
  mediaInfoText,
  stripBlocker,
  tagGroups,
  wavePath,
} from "../../lib/mediaInfo";
import { fileName, kindOf } from "../../lib/routing";
import { activeDocument, useDocuments } from "../../state/documents";
import { assetFile, useAsset, useMediaInfo } from "../../state/media";
import { useRecent } from "../../state/recent";
import { useUi } from "../../state/ui";
import { quickTool } from "../../workspaces/quickTools";
import { workspaceInfo } from "../../workspaces/workspaces";

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

/** details about the active document */
export function InfoPanel() {
  const doc = useDocuments(activeDocument);
  const summary = useDocuments((s) => (s.activeId ? s.summaries[s.activeId] : undefined));
  const recent = useRecent((s) => s.files);
  // a vellum project is not a media file; probing it would only report an error
  const probed = doc?.path && kindOf(doc.path)?.kind !== "project" ? doc.path : null;
  const probe = useMediaInfo(probed);

  if (!doc) {
    return (
      <EmptyState icon={Info} title="No document open" compact>
        Open a file to see its type, size, codecs and metadata here.
      </EmptyState>
    );
  }

  const size = recent?.find((f) => f.path === doc.path)?.sizeBytes ?? null;
  const tool = doc.tool ? quickTool(doc.tool) : undefined;
  const rows: [string, string][] = [["Workspace", workspaceInfo(doc.workspace).label]];
  if (!doc.path) rows.push(["Type", tool ? "Quick tool" : "New"]);
  if (doc.path && probe?.state !== "ready") {
    rows.push(["Type", kindOf(doc.path)?.label ?? "Unknown"]);
    if (size !== null) rows.push(["Size", formatBytes(size)]);
  }
  if (tool) rows.push(["Tool", tool.title]);
  if (doc.inputs.length > 0) rows.push(["Files", doc.inputs.map(fileName).join(", ")]);
  rows.push(["Changes", doc.dirty ? "Not saved yet" : "Saved"]);
  if (summary?.undoName) rows.push(["Last step", summary.undoName]);

  return (
    <section aria-label="Info" className="flex flex-col gap-3 overflow-y-auto p-3">
      {doc.path && probe?.state === "ready" && <Preview info={probe.info} />}
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
      <Rows rows={rows} />
      {doc.path && probe?.state === "loading" && (
        <div className="flex flex-col gap-1.5" aria-label="Reading the file">
          <Skeleton className="h-3 w-2/3" />
          <Skeleton className="h-3 w-1/2" />
          <Skeleton className="h-3 w-3/5" />
        </div>
      )}
      {doc.path && probe?.state === "failed" && (
        <p className="rounded-md bg-danger/10 px-2.5 py-2 text-xs text-danger">{probe.error}</p>
      )}
      {probe?.state === "ready" && <MediaDetails info={probe.info} />}
    </section>
  );
}

function Rows({ rows }: { rows: [string, string][] }) {
  return (
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
  );
}

function MediaDetails({ info }: { info: MediaInfo }) {
  const blocker = stripBlocker(info);
  const groups = tagGroups(info.tags);
  return (
    <>
      {info.hasLocation && (
        <p className="flex items-start gap-1.5 rounded-md bg-warning/10 px-2.5 py-2 text-xs text-warning">
          <MapPin className="mt-px size-3.5 shrink-0" />
          This file records where it was made. Strip metadata before sharing it if that is private.
        </p>
      )}
      {infoSections(info).map((section) => (
        <div key={section.title} className="flex flex-col gap-1.5">
          <h4 className="text-2xs font-semibold tracking-wider text-fg-subtle uppercase">
            {section.title}
          </h4>
          <Rows rows={section.rows} />
        </div>
      ))}
      {groups.length > 0 && (
        <details className="group flex flex-col gap-1.5">
          <summary className="cursor-default text-2xs font-semibold tracking-wider text-fg-subtle uppercase select-none hover:text-fg-muted">
            Metadata ({info.tags.length})
          </summary>
          <div className="mt-1.5 flex flex-col gap-2">
            {groups.map(([group, tags]) => (
              <div key={group} className="flex flex-col gap-1">
                <span className="text-2xs text-fg-subtle">{group}</span>
                <dl className="flex flex-col gap-1 text-xs">
                  {tags.map((tag, index) => (
                    // names repeat in some files, so the index keeps keys unique
                    <div key={`${tag.key}-${String(index)}`} className="flex flex-col">
                      <dt className="truncate text-fg-subtle" title={tag.key}>
                        {tag.key}
                      </dt>
                      <dd className="break-all text-fg-muted select-text">{tag.value}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            ))}
          </div>
        </details>
      )}
      <div className="flex flex-wrap gap-2 pt-1">
        <Button
          size="sm"
          icon={<ClipboardCopy className="size-3.5" />}
          tooltip="Copy these details as text, for a bug report or a chat"
          onClick={() => {
            copyText(mediaInfoText(info), "file info");
          }}
        >
          Copy info
        </Button>
        <Button
          size="sm"
          icon={<Eraser className="size-3.5" />}
          tooltip="Save a copy without camera details, location, comments and tags. The original is not changed."
          disabled={!inDesktopApp || blocker !== null}
          onClick={() => {
            ipc
              .stripMetadata(info.path)
              .then(() => {
                useUi.getState().toast({
                  level: "info",
                  title: "Stripping metadata",
                  body: "The copy will be saved next to the original.",
                });
              })
              .catch((err: unknown) => {
                useUi.getState().toast({
                  level: "error",
                  title: "Metadata could not be stripped",
                  body: String(err),
                });
              });
          }}
        >
          Strip metadata
        </Button>
      </div>
      {blocker && <p className="text-xs text-fg-subtle">{blocker}</p>}
    </>
  );
}

/** a thumbnail, plus a filmstrip for videos or a waveform for audio */
function Preview({ info }: { info: MediaInfo }) {
  const visual = info.kind !== "audio" && info.kind !== "unknown";
  const thumb = useAsset("thumbnail", info.path);
  const strip = useAsset("filmstrip", info.kind === "video" ? info.path : null);
  const wave = useAsset("waveform", info.audio.length > 0 ? info.path : null);
  const thumbFile = assetFile(thumb);
  const stripFile = assetFile(strip);

  return (
    <div className="flex flex-col gap-1.5">
      {(visual || thumbFile) && (
        <div className="grain flex h-28 items-center justify-center overflow-hidden rounded-md border border-line bg-raised">
          {thumbFile ? (
            <img
              src={convertFileSrc(thumbFile)}
              alt={`Preview of ${info.name}`}
              draggable={false}
              className="size-full object-contain"
            />
          ) : thumb?.state === "failed" ? (
            <span className="px-3 text-center text-xs text-fg-subtle">
              No preview: {thumb.error}
            </span>
          ) : (
            <Skeleton className="size-full rounded-none" />
          )}
        </div>
      )}
      {info.kind === "video" &&
        (stripFile ? (
          <img
            src={convertFileSrc(stripFile)}
            alt="Frames from across the video"
            draggable={false}
            className="w-full rounded-sm border border-line"
          />
        ) : strip?.state === "pending" || strip === undefined ? (
          <Skeleton className="h-6 w-full" />
        ) : null)}
      {info.audio.length > 0 && wave?.state === "ready" && <Waveform path={info.path} />}
      {info.audio.length > 0 && (wave === undefined || wave.state === "pending") && (
        <Skeleton className="h-10 w-full" />
      )}
    </div>
  );
}

const WAVE_COLUMNS = 240;

function Waveform({ path }: { path: string }) {
  const [overview, setOverview] = useState<WaveformOverview | null>(null);
  useEffect(() => {
    let live = true;
    void ipc.waveformOverview(path, WAVE_COLUMNS).then((view) => {
      if (live) setOverview(view);
    });
    return () => {
      live = false;
    };
  }, [path]);
  if (!overview) return <Skeleton className="h-10 w-full" />;
  return (
    <svg
      role="img"
      aria-label="Waveform of the audio"
      viewBox={`0 0 ${String(overview.columns)} 40`}
      preserveAspectRatio="none"
      className="h-10 w-full text-accent"
    >
      <path d={wavePath(overview)} fill="currentColor" />
    </svg>
  );
}
