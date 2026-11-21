import { Dialog as RadixDialog } from "radix-ui";
import { Search, X } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { AppInfo } from "../../bindings/AppInfo";
import type { AppSettings } from "../../bindings/AppSettings";
import type { Theme } from "../../bindings/Theme";
import { Button, IconButton } from "../../components/Button";
import { Switch } from "../../components/Switch";
import { Tooltip } from "../../components/Tooltip";
import { accentSwatches, isHexColour } from "../../lib/color";
import { rank } from "../../lib/fuzzy";
import { formatBytes } from "../../lib/format";
import { inDesktopApp, ipc, type DeepPartial } from "../../lib/ipc";
import { useMedia } from "../../state/media";
import { useSettings } from "../../state/settings";
import { useUi, type SettingsSection } from "../../state/ui";
import { sectionTitles, settingsCatalog, type SettingEntry } from "./catalog";
import { ShortcutEditor } from "./ShortcutEditor";

const sections = Object.keys(sectionTitles) as SettingsSection[];

/** the settings window: sections on the left, a search that covers every setting */
export function SettingsDialog() {
  const section = useUi((s) => s.settingsSection);
  const initialQuery = useUi((s) => s.settingsQuery);
  const close = useUi((s) => s.closeSettings);
  return (
    <RadixDialog.Root
      open={section !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="fixed inset-0 z-[var(--z-modal)] animate-fade-in bg-scrim backdrop-blur-[2px]" />
        <RadixDialog.Content
          aria-describedby={undefined}
          className="fixed top-1/2 left-1/2 z-[var(--z-modal)] flex h-[min(42rem,calc(100vh-3rem))] w-[min(58rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 animate-pop-in overflow-hidden rounded-xl bg-panel text-fg shadow-pop"
        >
          {section && (
            <SettingsBody
              key={`${section}:${initialQuery}`}
              start={section}
              initialQuery={initialQuery}
            />
          )}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}

function SettingsBody({ start, initialQuery }: { start: SettingsSection; initialQuery: string }) {
  const [section, setSection] = useState<SettingsSection>(start);
  const [query, setQuery] = useState(initialQuery);
  const settings = useSettings((s) => s.settings);
  const error = useSettings((s) => s.error);
  const searching = query.trim() !== "";

  const matches = useMemo(
    () =>
      rank(query, settingsCatalog, (e) => ({
        title: e.label,
        keywords: `${e.description} ${e.keywords ?? ""} ${sectionTitles[e.section]}`,
      })).map((r) => r.item),
    [query],
  );

  return (
    <>
      <nav
        aria-label="Settings sections"
        className="flex w-56 shrink-0 flex-col gap-0.5 border-r border-line bg-canvas/40 p-3"
      >
        <RadixDialog.Title className="px-2 pt-1 pb-3 text-md font-semibold tracking-tight">
          Settings
        </RadixDialog.Title>
        <label className="mb-2 flex h-(--row) items-center gap-2 rounded-md border border-line bg-raised px-2 focus-within:border-accent-line">
          <Search className="size-3.5 text-fg-subtle" />
          <input
            autoFocus
            aria-label="Search settings"
            placeholder="Search settings"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
            }}
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-fg-subtle focus-visible:shadow-none"
          />
        </label>
        {sections.map((id) => (
          <button
            key={id}
            type="button"
            aria-current={!searching && section === id ? "page" : undefined}
            onClick={() => {
              setQuery("");
              setSection(id);
            }}
            className={`flex h-(--row) items-center rounded-md px-2 text-left text-sm transition-colors duration-(--motion-fast) ${
              !searching && section === id
                ? "bg-elevated text-fg"
                : "text-fg-muted hover:bg-elevated/60 hover:text-fg"
            }`}
          >
            {sectionTitles[id]}
          </button>
        ))}
      </nav>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-12 shrink-0 items-center justify-between border-b border-line px-5">
          <h3 className="text-sm font-semibold">
            {searching ? `Results for “${query.trim()}”` : sectionTitles[section]}
          </h3>
          <RadixDialog.Close asChild>
            <IconButton label="Close settings" shortcut="Escape" icon={<X className="size-4" />} />
          </RadixDialog.Close>
        </div>
        {error && (
          <p
            role="alert"
            className="mx-5 mt-3 rounded-md bg-danger/10 px-3 py-2 text-xs text-danger"
          >
            Settings could not be saved: {error}. Try again, or restart Vellum if it keeps
            happening.
          </p>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {!settings ? (
            <p className="text-sm text-fg-muted">Settings are loading.</p>
          ) : searching ? (
            <SearchResults matches={matches} query={query} settings={settings} />
          ) : section === "shortcuts" ? (
            <ShortcutEditor query="" />
          ) : section === "about" ? (
            <About />
          ) : section === "media" ? (
            <div className="flex flex-col gap-6">
              <Rows
                entries={settingsCatalog.filter((e) => e.section === "media")}
                settings={settings}
              />
              <MediaStatusPanel />
            </div>
          ) : (
            <Rows
              entries={settingsCatalog.filter((e) => e.section === section)}
              settings={settings}
            />
          )}
        </div>
      </div>
    </>
  );
}

function SearchResults({
  matches,
  query,
  settings,
}: {
  matches: SettingEntry[];
  query: string;
  settings: AppSettings;
}) {
  const plain = matches.filter((m) => m.section !== "about");
  return (
    <div className="flex flex-col gap-6">
      {plain.length > 0 && <Rows entries={plain} settings={settings} showSection />}
      <section className="flex flex-col gap-2">
        <h4 className="text-xs font-semibold tracking-wide text-fg-muted uppercase">Shortcuts</h4>
        <ShortcutEditor query={query} />
      </section>
    </div>
  );
}

function Rows({
  entries,
  settings,
  showSection = false,
}: {
  entries: SettingEntry[];
  settings: AppSettings;
  showSection?: boolean;
}) {
  return (
    <div className="flex flex-col divide-y divide-line">
      {entries.map((entry) => (
        <div key={entry.id} className="flex items-center gap-6 py-3.5">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <label htmlFor={`setting-${entry.id}`} className="text-sm font-medium">
              {entry.label}
            </label>
            <p className="text-xs text-fg-muted">
              {showSection && (
                <span className="text-fg-subtle">{sectionTitles[entry.section]} · </span>
              )}
              {entry.description}
            </p>
          </div>
          <div className="shrink-0">
            <Control entry={entry} settings={settings} />
          </div>
        </div>
      ))}
    </div>
  );
}

function Control({ entry, settings }: { entry: SettingEntry; settings: AppSettings }) {
  const update = useSettings((s) => s.update);
  const patch = (value: DeepPartial<AppSettings>) => void update(value);
  const id = `setting-${entry.id}`;
  const look = settings.appearance;

  switch (entry.id) {
    case "theme":
      return (
        <Segmented
          id={id}
          label="Theme"
          value={look.theme}
          options={[
            ["dark", "Dark"],
            ["light", "Light"],
            ["high-contrast", "High contrast"],
          ]}
          onChange={(theme: Theme) => {
            patch({ appearance: { theme } });
          }}
        />
      );
    case "accent":
      return (
        <AccentPicker
          id={id}
          value={look.accent}
          onChange={(accent) => {
            patch({ appearance: { accent } });
          }}
        />
      );
    case "uiScale":
      return (
        <Segmented
          id={id}
          label="Interface size"
          value={String(look.uiScale)}
          options={[
            ["0.85", "85%"],
            ["0.9", "90%"],
            ["1", "100%"],
            ["1.1", "110%"],
            ["1.25", "125%"],
            ["1.5", "150%"],
          ]}
          onChange={(value) => {
            patch({ appearance: { uiScale: Number(value) } });
          }}
        />
      );
    case "compact":
      return (
        <Switch
          id={id}
          label="Compact mode"
          checked={look.compact}
          onCheckedChange={(compact) => {
            patch({ appearance: { compact } });
          }}
        />
      );
    case "reducedMotion":
      return (
        <Switch
          id={id}
          label="Reduce motion"
          checked={look.reducedMotion}
          onCheckedChange={(reducedMotion) => {
            patch({ appearance: { reducedMotion } });
          }}
        />
      );
    case "preset":
      return (
        <Button
          tooltip="Open the shortcut editor"
          onClick={() => {
            useUi.getState().openSettings("shortcuts");
          }}
        >
          Edit shortcuts
        </Button>
      );
    case "autosaveEnabled":
      return (
        <Switch
          id={id}
          label="Autosave"
          checked={settings.autosave.enabled}
          onCheckedChange={(enabled) => {
            patch({ autosave: { enabled } });
          }}
        />
      );
    case "autosaveInterval":
      return (
        <NumberField
          id={id}
          value={settings.autosave.intervalSecs}
          min={5}
          max={3600}
          unit="seconds"
          onChange={(intervalSecs) => {
            patch({ autosave: { intervalSecs } });
          }}
        />
      );
    case "maxCpuJobs":
      return (
        <NumberField
          id={id}
          value={settings.jobs.maxCpuJobs}
          min={1}
          max={64}
          onChange={(maxCpuJobs) => {
            patch({ jobs: { maxCpuJobs } });
          }}
        />
      );
    case "maxGpuJobs":
      return (
        <NumberField
          id={id}
          value={settings.jobs.maxGpuJobs}
          min={1}
          max={16}
          onChange={(maxGpuJobs) => {
            patch({ jobs: { maxGpuJobs } });
          }}
        />
      );
    case "maxIoJobs":
      return (
        <NumberField
          id={id}
          value={settings.jobs.maxIoJobs}
          min={1}
          max={64}
          onChange={(maxIoJobs) => {
            patch({ jobs: { maxIoJobs } });
          }}
        />
      );
    case "maxSteps":
      return (
        <NumberField
          id={id}
          value={settings.history.maxSteps}
          min={10}
          max={10000}
          unit="steps"
          onChange={(maxSteps) => {
            patch({ history: { maxSteps } });
          }}
        />
      );
    case "maxMemory":
      return (
        <NumberField
          id={id}
          value={settings.history.maxMemoryMb}
          min={64}
          max={65536}
          unit="MB"
          onChange={(maxMemoryMb) => {
            patch({ history: { maxMemoryMb } });
          }}
        />
      );
    case "ffmpeg":
      return (
        <TextField
          id={id}
          value={settings.media.ffmpegPath}
          placeholder="Find automatically"
          onChange={(ffmpegPath) => {
            patch({ media: { ffmpegPath } });
          }}
        />
      );
    case "hardwareAcceleration":
      return (
        <Switch
          id={id}
          label="Hardware acceleration"
          checked={settings.media.hardwareAcceleration}
          onCheckedChange={(hardwareAcceleration) => {
            patch({ media: { hardwareAcceleration } });
          }}
        />
      );
    case "cacheLimit":
      return (
        <NumberField
          id={id}
          value={settings.media.cacheLimitMb}
          min={100}
          max={1048576}
          unit="MB"
          onChange={(cacheLimitMb) => {
            patch({ media: { cacheLimitMb } });
          }}
        />
      );
    case "maxImageMemory":
      return (
        <NumberField
          id={id}
          value={settings.media.maxImageMb}
          min={256}
          max={65536}
          unit="MB"
          onChange={(maxImageMb) => {
            patch({ media: { maxImageMb } });
          }}
        />
      );
    default:
      return null;
  }
}

function TextField({
  id,
  value,
  placeholder,
  onChange,
}: {
  id: string;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }
  const commit = () => {
    if (draft.trim() !== value) onChange(draft.trim());
  };
  return (
    <input
      id={id}
      value={draft}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(event) => {
        setDraft(event.target.value);
      }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
      }}
      className="h-(--row-sm) w-56 rounded-md border border-line-strong bg-raised px-2 font-mono text-xs text-fg outline-none placeholder:font-sans placeholder:text-fg-subtle focus:border-accent-line"
    />
  );
}

/** what the media core found: ffmpeg, the graphics card's encoders and the cache */
function MediaStatusPanel() {
  const status = useMedia((s) => s.status);
  const [cleaning, setCleaning] = useState(false);
  // the cache grows quietly as thumbnails are made, so read it fresh when this opens
  useEffect(() => {
    if (inDesktopApp) void ipc.mediaStatus().then(useMedia.getState().setStatus);
  }, []);
  if (!inDesktopApp) {
    return <p className="text-sm text-fg-muted">Media details show in the desktop app.</p>;
  }
  if (!status) return <p className="text-sm text-fg-muted">Checking media tools.</p>;

  const byApi = new Map<string, string[]>();
  for (const encoder of status.hardware?.encoders ?? []) {
    const codecs = byApi.get(encoder.label) ?? [];
    codecs.push(
      encoder.codec === "hevc" ? "H.265" : encoder.codec.toUpperCase().replace("H264", "H.264"),
    );
    byApi.set(encoder.label, codecs);
  }
  const hardware = status.checkingHardware
    ? "Checking your graphics card."
    : byApi.size > 0
      ? [...byApi].map(([api, codecs]) => `${api}: ${codecs.join(", ")}`).join("; ")
      : status.hardware
        ? "None found. Video is encoded on the processor, which is slower but looks the same."
        : "Not checked yet.";
  const cache = status.cache;

  const rows: [string, ReactNode][] = [
    [
      "FFmpeg",
      status.ffmpeg ? (
        <span title={status.ffmpeg.ffmpegPath}>
          Version {status.ffmpeg.version} at {status.ffmpeg.ffmpegPath}
        </span>
      ) : (
        <span className="text-danger">
          {status.ffmpegError ?? "Not found."} Images still open, video and audio need it.
        </span>
      ),
    ],
    ["Video hardware", hardware],
    [
      "Cache",
      cache
        ? `${formatBytes(cache.bytes)} of ${formatBytes(cache.limitBytes)} used, ${String(cache.entries)} items`
        : (status.cacheError ?? "Off"),
    ],
  ];

  return (
    <section aria-label="Media status" className="flex flex-col gap-3">
      <dl className="grid grid-cols-[8rem_1fr] gap-y-2 text-sm">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-fg-muted">{label}</dt>
            <dd className="min-w-0 text-xs leading-5 break-words text-fg-muted select-text">
              {value}
            </dd>
          </div>
        ))}
      </dl>
      <div className="flex gap-2">
        <Button
          tooltip="Try the graphics card's video encoders again, for example after a driver update"
          disabled={status.checkingHardware || !status.ffmpeg}
          onClick={() => {
            void ipc.recheckHardware();
          }}
        >
          Check hardware again
        </Button>
        <Button
          variant="danger"
          tooltip="Delete every cached thumbnail and waveform. They are made again when needed."
          disabled={cleaning || !cache || cache.entries === 0}
          onClick={() => {
            setCleaning(true);
            ipc
              .cleanMediaCache()
              .then((after) => {
                useUi.getState().toast({
                  level: "success",
                  title: "Cache cleaned",
                  body: `${formatBytes(cache?.bytes ?? 0)} freed.`,
                });
                useMedia.getState().setStatus({ ...status, cache: after });
              })
              .catch((err: unknown) => {
                useUi.getState().toast({
                  level: "error",
                  title: "The cache could not be cleaned",
                  body: String(err),
                });
              })
              .finally(() => {
                setCleaning(false);
              });
          }}
        >
          Clean cache
        </Button>
      </div>
    </section>
  );
}

function Segmented<T extends string>({
  id,
  label,
  value,
  options,
  onChange,
}: {
  id: string;
  label: string;
  value: T;
  options: [T, string][];
  onChange: (value: T) => void;
}) {
  return (
    <div
      id={id}
      role="radiogroup"
      aria-label={label}
      className="flex rounded-md border border-line-strong bg-canvas/50 p-0.5"
    >
      {options.map(([option, text]) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={value === option}
          onClick={() => {
            onChange(option);
          }}
          className={`h-6 rounded-sm px-2.5 text-xs transition-colors duration-(--motion-fast) ${
            value === option ? "bg-elevated text-fg shadow-1" : "text-fg-muted hover:text-fg"
          }`}
        >
          {text}
        </button>
      ))}
    </div>
  );
}

function AccentPicker({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const [custom, setCustom] = useState(value);
  // follow changes made elsewhere, like a swatch click or the palette
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setCustom(value);
  }
  return (
    <div id={id} className="flex items-center gap-1.5">
      {accentSwatches.map((swatch) => (
        <Tooltip key={swatch.value} text={swatch.name}>
          <button
            type="button"
            aria-label={`${swatch.name} accent`}
            aria-pressed={value.toLowerCase() === swatch.value}
            onClick={() => {
              onChange(swatch.value);
            }}
            style={{ background: swatch.value }}
            className={`size-5 rounded-full transition-transform duration-(--motion-fast) hover:scale-110 ${
              value.toLowerCase() === swatch.value
                ? "ring-2 ring-fg ring-offset-2 ring-offset-panel"
                : ""
            }`}
          />
        </Tooltip>
      ))}
      <Tooltip text="Any colour, as #rrggbb">
        <input
          aria-label="Custom accent colour"
          value={custom}
          maxLength={7}
          onChange={(event) => {
            setCustom(event.target.value);
            if (isHexColour(event.target.value)) onChange(event.target.value.toLowerCase());
          }}
          className={`ml-1 h-6 w-20 rounded-md border bg-raised px-1.5 font-mono text-xs outline-none ${isHexColour(custom) ? "border-line-strong" : "border-danger"}`}
        />
      </Tooltip>
    </div>
  );
}

function NumberField({
  id,
  value,
  min,
  max,
  unit,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  unit?: string;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(String(value));
  }
  const commit = () => {
    const n = Math.round(Number(draft));
    if (Number.isFinite(n) && n !== value) onChange(Math.min(max, Math.max(min, n)));
    else setDraft(String(value));
  };
  return (
    <span className="flex items-center gap-2 text-xs text-fg-muted">
      <input
        id={id}
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
        }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
        }}
        className="h-(--row-sm) w-20 rounded-md border border-line-strong bg-raised px-2 text-right text-sm text-fg tabular outline-none focus:border-accent-line"
      />
      {unit}
    </span>
  );
}

function About() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    if (inDesktopApp) void ipc.appInfo().then(setInfo);
  }, []);
  const rows: [string, ReactNode][] = [
    ["Version", info?.version ?? __APP_VERSION__],
    ["System", info?.os ?? "Browser preview"],
    ["Settings file", info?.settingsPath ?? "Only in the desktop app"],
  ];
  return (
    <div className="flex flex-col gap-6">
      <dl className="grid grid-cols-[10rem_1fr] gap-y-2 text-sm">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-fg-muted">{label}</dt>
            <dd className="min-w-0 truncate font-mono text-xs leading-6 select-text">{value}</dd>
          </div>
        ))}
      </dl>
      <div>
        <Button
          variant="danger"
          tooltip="Put every setting back to how Vellum came. Your files are not touched."
          disabled={!inDesktopApp}
          onClick={() => {
            void ipc.resetSettings().then((settings) => {
              useSettings.getState().replace(settings);
            });
          }}
        >
          Reset all settings
        </Button>
      </div>
    </div>
  );
}
