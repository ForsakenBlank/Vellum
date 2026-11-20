import { convertFileSrc } from "@tauri-apps/api/core";
import { Clock, FolderOpen, Pin, PinOff, Upload, X } from "lucide-react";
import { useState } from "react";

import type { RecentFile } from "../bindings/RecentFile";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Kbd } from "../components/Kbd";
import { Skeleton } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { formatAgo, formatBytes } from "../lib/format";
import { inDesktopApp, ipc } from "../lib/ipc";
import { kindOf } from "../lib/routing";
import { useCommands } from "../state/commands";
import { assetFile, useAsset } from "../state/media";
import { useRecent } from "../state/recent";
import { useUi } from "../state/ui";
import { quickTools, type QuickTool } from "../workspaces/quickTools";
import { workspaceInfo } from "../workspaces/workspaces";
import { openPaths, openTool, pickAndOpen } from "./actions";

export function Home() {
  const files = useRecent((s) => s.files);
  const pinned = files?.filter((f) => f.pinned) ?? [];
  const recent = files?.filter((f) => !f.pinned) ?? [];

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-6xl flex-col gap-10 px-8 pt-8 pb-16">
        <DropHero />

        <section aria-labelledby="quick-tools-heading" className="flex flex-col gap-3">
          <SectionTitle
            id="quick-tools-heading"
            title="Quick tools"
            hint="Drop a file on a card to start there"
          />
          <div className="grid grid-cols-[repeat(auto-fit,minmax(9.5rem,1fr))] gap-3">
            {quickTools.map((tool) => (
              <ToolCard key={tool.id} tool={tool} />
            ))}
          </div>
        </section>

        {pinned.length > 0 && (
          <section aria-labelledby="pinned-heading" className="flex flex-col gap-3">
            <SectionTitle id="pinned-heading" title="Pinned" />
            <FileGrid files={pinned} />
          </section>
        )}

        <section aria-labelledby="recent-heading" className="flex flex-col gap-3">
          <SectionTitle id="recent-heading" title="Recent" />
          {files === null ? (
            <div
              className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3"
              aria-busy
              aria-label="Loading recent files"
            >
              {Array.from({ length: 6 }, (_, i) => (
                <div key={i} className="flex flex-col gap-2">
                  <Skeleton className="aspect-[4/3] w-full rounded-lg" />
                  <Skeleton className="h-3.5 w-3/4" />
                  <Skeleton className="h-3 w-1/2" />
                </div>
              ))}
            </div>
          ) : recent.length === 0 ? (
            <div className="rounded-xl border border-dashed border-line">
              <EmptyState icon={Clock} title="No recent files yet">
                Files you open show up here, so getting back to them is one click. Pin the ones you
                keep coming back to.
              </EmptyState>
            </div>
          ) : (
            <FileGrid files={recent} />
          )}
        </section>
      </div>
    </div>
  );
}

function SectionTitle({ id, title, hint }: { id: string; title: string; hint?: string }) {
  return (
    <div className="flex items-baseline justify-between">
      <h2 id={id} className="text-xs font-semibold tracking-wide text-fg-muted uppercase">
        {title}
      </h2>
      {hint && <span className="text-xs text-fg-subtle">{hint}</span>}
    </div>
  );
}

function DropHero() {
  const drop = useUi((s) => s.drop);
  const dropTool = useUi((s) => s.dropTool);
  const openChord = useCommands((s) => s.keymap?.bindings["file.open"]?.[0]);
  const hot = drop !== null && dropTool === null;
  return (
    <section
      aria-label="Drop zone"
      className={`grain relative flex flex-col items-center gap-4 overflow-hidden rounded-2xl border px-8 py-12 text-center transition-colors duration-(--motion-base) ${
        hot ? "border-accent bg-accent-soft" : "border-line bg-panel"
      }`}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute -top-24 left-1/2 h-48 w-[36rem] -translate-x-1/2 rounded-full bg-accent opacity-[0.07] blur-3xl"
      />
      <div className="flex size-12 items-center justify-center rounded-xl border border-line-strong bg-raised text-accent-text shadow-1">
        <Upload className="size-5" strokeWidth={1.7} />
      </div>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-xl font-semibold tracking-tight">Drop anything to start</h1>
        <p className="max-w-md text-sm text-fg-muted">
          Images, videos, audio, GIFs, SVGs and PDFs open in the right workspace. Drop on a quick
          tool below to go straight there.
        </p>
      </div>
      <div className="flex items-center gap-2">
        <Button
          variant="primary"
          size="lg"
          tooltip="Pick files to open"
          command="file.open"
          icon={<FolderOpen className="size-4" />}
          disabled={!inDesktopApp}
          onClick={() => void pickAndOpen()}
        >
          Open files
        </Button>
        {openChord && (
          <span className="flex items-center gap-1.5 text-xs text-fg-subtle">
            or press <Kbd chord={openChord} />
          </span>
        )}
      </div>
    </section>
  );
}

function ToolCard({ tool }: { tool: QuickTool }) {
  const dropTool = useUi((s) => s.dropTool);
  const drop = useUi((s) => s.drop);
  const hovered = dropTool === tool.id;
  const rejected = hovered && drop?.action === "reject";
  return (
    <Tooltip text={`${tool.blurb} Arrives in phase ${tool.phase}.`} command={tool.command}>
      <button
        type="button"
        data-drop-tool={tool.id}
        onClick={() => void openTool(tool.id)}
        className={`group relative flex h-36 flex-col justify-between overflow-hidden rounded-xl border p-4 text-left transition-[border-color,background-color,transform,box-shadow] duration-(--motion-base) ease-calm ${
          rejected
            ? "border-danger bg-danger/10"
            : hovered
              ? "-translate-y-0.5 border-accent bg-accent-soft shadow-2"
              : "border-line bg-panel hover:-translate-y-0.5 hover:border-line-strong hover:bg-raised hover:shadow-2"
        }`}
      >
        <span
          className={`flex size-9 items-center justify-center rounded-lg transition-colors duration-(--motion-base) ${
            hovered && !rejected
              ? "bg-accent text-accent-fg"
              : "bg-raised text-fg-muted group-hover:text-accent-text"
          }`}
        >
          <tool.icon className="size-[1.1rem]" strokeWidth={1.7} />
        </span>
        <span className="flex flex-col gap-0.5">
          <span className="text-base font-semibold tracking-tight">{tool.title}</span>
          <span className="line-clamp-2 text-xs text-fg-muted">
            {hovered && drop ? drop.title : tool.blurb}
          </span>
        </span>
      </button>
    </Tooltip>
  );
}

function FileGrid({ files }: { files: RecentFile[] }) {
  return (
    <ul className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
      {files.map((file) => (
        <FileCard key={file.path} file={file} />
      ))}
    </ul>
  );
}

function FileCard({ file }: { file: RecentFile }) {
  const row = kindOf(file.path);
  const space = row ? workspaceInfo(row.workspace) : undefined;
  const fail = (err: unknown) => {
    useUi.getState().toast({ level: "error", title: "That did not work", body: String(err) });
  };
  const meta = [
    row?.label,
    file.sizeBytes !== null ? formatBytes(file.sizeBytes) : null,
    formatAgo(file.openedAtMs),
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <li className="group relative">
      <Tooltip text={`Open ${file.path}`}>
        <button
          type="button"
          onClick={() => void openPaths([file.path])}
          className="flex w-full flex-col gap-2 rounded-xl p-1.5 text-left transition-colors duration-(--motion-fast) hover:bg-raised"
        >
          <Thumbnail file={file} />
          <span className="flex flex-col gap-0.5 px-1">
            <span className="truncate text-sm font-medium" title={file.name}>
              {file.name}
            </span>
            <span className="flex items-center gap-1 truncate text-xs text-fg-subtle tabular">
              {space && <space.icon className="size-3 shrink-0" />}
              {meta}
            </span>
          </span>
        </button>
      </Tooltip>
      <div className="absolute top-3 right-3 flex gap-1 opacity-0 transition-opacity duration-(--motion-fast) group-focus-within:opacity-100 group-hover:opacity-100">
        <IconButton
          size="sm"
          label={file.pinned ? "Unpin" : "Pin to the top of home"}
          icon={file.pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />}
          className="bg-elevated/90 shadow-1 backdrop-blur"
          onClick={() => {
            ipc.pinRecent(file.path, !file.pinned).catch(fail);
          }}
        />
        <IconButton
          size="sm"
          label="Remove from recent files. The file itself is not touched."
          icon={<X className="size-3.5" />}
          className="bg-elevated/90 shadow-1 backdrop-blur"
          onClick={() => {
            ipc.removeRecent(file.path).catch(fail);
          }}
        />
      </div>
      {file.pinned && (
        <Pin
          aria-label="Pinned"
          className="absolute top-3 left-3 size-3.5 text-accent drop-shadow"
        />
      )}
    </li>
  );
}

function Thumbnail({ file }: { file: RecentFile }) {
  const [broken, setBroken] = useState<string | null>(null);
  const row = kindOf(file.path);
  const Icon = row ? workspaceInfo(row.workspace).icon : FolderOpen;
  const asset = useAsset("thumbnail", inDesktopApp ? file.path : null);
  const thumb = assetFile(asset);
  const shown = thumb !== null && thumb !== broken ? thumb : null;

  return (
    <div className="grain relative aspect-[4/3] w-full overflow-hidden rounded-lg border border-line bg-raised">
      {shown ? (
        <img
          src={convertFileSrc(shown)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => {
            setBroken(shown);
          }}
          className="checker size-full object-cover"
        />
      ) : asset?.state === "pending" ? (
        <Skeleton className="size-full rounded-none" />
      ) : (
        <div className="flex size-full flex-col items-center justify-center gap-2 bg-gradient-to-br from-elevated to-raised text-fg-subtle">
          <Icon className="size-6" strokeWidth={1.4} />
          <span className="font-mono text-2xs tracking-wider uppercase">
            {row ? (file.path.split(".").pop() ?? "") : "file"}
          </span>
        </div>
      )}
    </div>
  );
}
