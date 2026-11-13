import { clsx } from "clsx";
import {
  Clock,
  FolderOpen,
  MoreHorizontal,
  Pin,
  PinOff,
  Search,
  Trash2,
  Upload,
} from "lucide-react";
import { useState, type ReactNode } from "react";

import type { RecentEntry } from "../bindings/RecentEntry";
import { useShortcut } from "../commands/registry";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Kbd } from "../components/Kbd";
import { Menu } from "../components/Menu";
import { MediaIcon } from "../components/MediaIcon";
import { Skeleton } from "../components/Skeleton";
import { formatAgo } from "../lib/format";
import { perform, useApp } from "../stores/app";
import { useRecent } from "../stores/recent";
import { openFiles, pickAndOpen } from "../stores/session";
import { useUi } from "../stores/ui";
import { useDrop } from "./drop";
import { quickTools, type QuickTool } from "./quickTools";

/** the start screen: quick tools, pinned files and recent files */
export function Home() {
  const status = useApp((state) => state.status);
  const entries = useRecent((state) => state.entries);
  const openKey = useShortcut("file.open");
  const paletteKey = useShortcut("app.palette");
  const ready = status === "ready";
  const pinned = entries?.filter((entry) => entry.pinned) ?? [];
  const recent = entries?.filter((entry) => !entry.pinned) ?? [];

  return (
    <div className="h-full overflow-y-auto bg-sunken">
      <div className="mx-auto flex max-w-5xl flex-col gap-9 px-8 pt-10 pb-16">
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Vellum</h1>
            <p className="mt-1 text-sm text-ink-soft">
              {status === "browser"
                ? "Open in a browser. Start the desktop app with pnpm tauri dev to open files."
                : status === "failed"
                  ? "Vellum could not reach its backend. Restart it to try again."
                  : "Drop a file anywhere to start, or pick a tool."}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              tip="Find any command, setting or file"
              shortcut={paletteKey}
              onClick={() => {
                useUi.getState().setPaletteOpen(true);
              }}
            >
              <Search className="size-4" aria-hidden />
              Search
              {paletteKey && <Kbd shortcut={paletteKey} className="ml-1" />}
            </Button>
            <Button
              variant="primary"
              tip="Choose files to open"
              shortcut={openKey}
              disabled={!ready}
              onClick={() => void pickAndOpen()}
            >
              <FolderOpen className="size-4" aria-hidden />
              Open file
            </Button>
          </div>
        </header>

        <section aria-labelledby="quick-tools-title">
          <SectionTitle id="quick-tools-title">Quick tools</SectionTitle>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-3">
            {quickTools.map((tool) => (
              <ToolCard key={tool.id} tool={tool} ready={ready} />
            ))}
          </div>
        </section>

        {pinned.length > 0 && (
          <section aria-labelledby="pinned-title">
            <SectionTitle id="pinned-title">Pinned</SectionTitle>
            <FileGrid entries={pinned} />
          </section>
        )}

        <section aria-labelledby="recent-title">
          <SectionTitle
            id="recent-title"
            action={
              recent.length > 0 && (
                <Button
                  tip="Forget recent files. Pinned files stay"
                  className="h-7 px-2 text-xs"
                  onClick={() => void perform((api) => api.recent.clear())}
                >
                  Clear
                </Button>
              )
            }
          >
            Recent
          </SectionTitle>
          {entries === null && status !== "browser" && status !== "failed" ? (
            <div
              aria-label="Loading recent files"
              className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-3"
            >
              {[0, 1, 2, 3].map((index) => (
                <div key={index} className="flex flex-col gap-2">
                  <Skeleton className="aspect-[4/3] w-full rounded-lg" />
                  <Skeleton className="h-3.5 w-3/4" />
                  <Skeleton className="h-3 w-1/2" />
                </div>
              ))}
            </div>
          ) : recent.length === 0 ? (
            <div className="rounded-xl border border-dashed border-line">
              <EmptyState
                icon={Clock}
                title="No recent files yet"
                body="Files you open show up here, so getting back to them is one click."
                compact
              />
            </div>
          ) : (
            <FileGrid entries={recent} />
          )}
        </section>
      </div>
    </div>
  );
}

function SectionTitle({
  id,
  children,
  action,
}: {
  id: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="mb-3 flex h-7 items-center justify-between">
      <h2 id={id} className="text-xs font-semibold tracking-wide text-ink-soft uppercase">
        {children}
      </h2>
      {action}
    </div>
  );
}

function ToolCard({ tool, ready }: { tool: QuickTool; ready: boolean }) {
  const key = `tool:${tool.id}`;
  const hovered = useDrop((state) => state.targetKey === key);
  const plan = useDrop((state) => (state.targetKey === key ? state.plan : null));
  const refused = hovered && plan !== null && plan.opens.length === 0;

  const onClick = () => {
    if (tool.takesFiles) {
      void pickAndOpen({ type: "tool", id: tool.id });
    } else {
      useApp.getState().setNotice(`${tool.title} arrives in phase ${tool.phase}.`);
    }
  };

  return (
    <button
      type="button"
      data-drop={key}
      disabled={!ready && tool.takesFiles}
      onClick={onClick}
      className={clsx(
        "v-focus group relative flex flex-col items-start gap-3 rounded-xl border bg-surface p-4 text-left",
        "transition-[border-color,background-color,transform,box-shadow] duration-(--v-base) ease-out-soft",
        "hover:-translate-y-0.5 hover:border-line-strong hover:shadow-pop disabled:pointer-events-none disabled:opacity-60",
        hovered && !refused && "-translate-y-0.5 border-accent bg-accent-soft shadow-pop",
        refused && "border-bad/60",
        !hovered && "border-line",
      )}
    >
      <span
        className={clsx(
          "flex size-9 items-center justify-center rounded-lg transition-colors duration-(--v-base)",
          hovered && !refused
            ? "bg-accent text-on-accent"
            : "bg-raised text-ink-soft group-hover:text-accent",
        )}
      >
        <tool.icon className="size-[1.125rem]" aria-hidden />
      </span>
      <span>
        <span className="block text-sm font-medium">{tool.title}</span>
        <span className="mt-0.5 block text-xs text-ink-soft">
          {hovered
            ? refused
              ? "Can't take this file"
              : `Drop to ${tool.dropText.toLowerCase()}`
            : tool.description}
        </span>
      </span>
    </button>
  );
}

function FileGrid({ entries }: { entries: RecentEntry[] }) {
  return (
    <ul className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-3">
      {entries.map((entry) => (
        <FileCard key={entry.path} entry={entry} />
      ))}
    </ul>
  );
}

function FileCard({ entry }: { entry: RecentEntry }) {
  const previewUrl = useApp((state) => state.previewUrl);
  const [broken, setBroken] = useState(false);
  const thumbnail =
    previewUrl && entry.exists && !broken && (entry.kind === "image" || entry.kind === "vector");

  return (
    <li className="group relative">
      <button
        type="button"
        title={entry.path}
        disabled={!entry.exists}
        onClick={() => void openFiles([entry.path])}
        className="v-focus flex w-full flex-col gap-2 rounded-lg text-left disabled:cursor-not-allowed"
      >
        <span
          className={clsx(
            "relative flex aspect-[4/3] w-full items-center justify-center overflow-hidden rounded-lg border border-line bg-surface",
            "transition-[border-color,box-shadow] duration-(--v-base) group-hover:border-line-strong group-hover:shadow-pop",
            !entry.exists && "opacity-50",
          )}
        >
          {thumbnail ? (
            <img
              src={previewUrl(entry.path)}
              alt=""
              loading="lazy"
              onError={() => {
                setBroken(true);
              }}
              className="size-full object-cover"
            />
          ) : (
            <MediaIcon kind={entry.kind} className="size-7 text-ink-faint" strokeWidth={1.5} />
          )}
          {!entry.exists && (
            <span className="absolute bottom-2 left-2 rounded bg-canvas/90 px-1.5 py-0.5 text-2xs text-ink-soft">
              Missing
            </span>
          )}
        </span>
        <span className="min-w-0 px-0.5">
          <span className="block truncate text-sm">{entry.name}</span>
          <span className="block truncate text-xs text-ink-faint">{formatAgo(entry.openedMs)}</span>
        </span>
      </button>
      <div className="absolute top-1.5 right-1.5 flex gap-1 opacity-0 transition-opacity duration-(--v-fast) group-focus-within:opacity-100 group-hover:opacity-100">
        <IconButton
          label={entry.pinned ? "Unpin" : "Pin to the top"}
          className="bg-canvas/80 backdrop-blur"
          onClick={() => void perform((api) => api.recent.pin(entry.path, !entry.pinned))}
        >
          {entry.pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />}
        </IconButton>
        <Menu.Root>
          <Menu.Trigger asChild>
            <IconButton label="More" className="bg-canvas/80 backdrop-blur">
              <MoreHorizontal className="size-3.5" />
            </IconButton>
          </Menu.Trigger>
          <Menu.Content align="end">
            <Menu.Item
              icon={<Upload className="size-3.5" />}
              disabled={!entry.exists}
              onSelect={() => void openFiles([entry.path])}
            >
              Open
            </Menu.Item>
            <Menu.Item
              icon={<FolderOpen className="size-3.5" />}
              disabled={!entry.exists}
              onSelect={() => void perform((api) => api.files.reveal(entry.path))}
            >
              Show in folder
            </Menu.Item>
            <Menu.Separator />
            <Menu.Item
              icon={<Trash2 className="size-3.5" />}
              onSelect={() => void perform((api) => api.recent.remove(entry.path))}
            >
              Remove from the list
            </Menu.Item>
          </Menu.Content>
        </Menu.Root>
      </div>
    </li>
  );
}
