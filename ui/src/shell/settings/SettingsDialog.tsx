import { clsx } from "clsx";
import { Contrast, Moon, Search, Sun } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { SettingsMap } from "../../bindings/SettingsMap";
import type { Theme } from "../../bindings/Theme";
import { PRESET_ORDER, PRESETS } from "../../commands/shortcuts";
import { Dialog } from "../../components/Dialog";
import { Segmented } from "../../components/Segmented";
import { Select } from "../../components/Select";
import { Slider } from "../../components/Slider";
import { Switch } from "../../components/Switch";
import { applyAppearance } from "../../lib/appearance";
import { fuzzyRank } from "../../lib/fuzzy";
import { updateSettings, useSettings } from "../../stores/settings";
import { type SettingsPane, useUi } from "../../stores/ui";
import { PANES, PRESET_SETTING, SETTINGS, type SettingEntry } from "./catalog";
import { ShortcutEditor } from "./ShortcutEditor";

const themes: { value: Theme; label: string; icon: ReactNode }[] = [
  { value: "dark", label: "Dark", icon: <Moon className="size-3.5" aria-hidden /> },
  { value: "light", label: "Light", icon: <Sun className="size-3.5" aria-hidden /> },
  {
    value: "highContrast",
    label: "High contrast",
    icon: <Contrast className="size-3.5" aria-hidden />,
  },
];

const ACCENTS = ["#7c6cff", "#3b82f6", "#14b8a6", "#22c55e", "#f59e0b", "#ef4444", "#ec4899"];

const AUTOSAVE = [
  { value: "0", label: "Off" },
  { value: "10", label: "Every 10 seconds" },
  { value: "30", label: "Every 30 seconds" },
  { value: "60", label: "Every minute" },
  { value: "120", label: "Every 2 minutes" },
  { value: "300", label: "Every 5 minutes" },
];

const RECENT = ["10", "20", "30", "50", "100"].map((value) => ({ value, label: value }));
const CPU_JOBS = [
  { value: "0", label: "Automatic" },
  ...["1", "2", "3", "4", "6", "8"].map((value) => ({ value, label: value })),
];
const GPU_JOBS = ["1", "2", "3", "4"].map((value) => ({ value, label: value }));

/** keeps a value from the list in the options, so an unusual stored number still shows */
function withValue(options: { value: string; label: string }[], value: number) {
  const text = String(value);
  return options.some((option) => option.value === text)
    ? options
    : [...options, { value: text, label: text }];
}

function Control({ id, sections }: { id: string; sections: SettingsMap }) {
  const { appearance, general, performance, shortcuts } = sections;
  // the value under the slider while it is dragged, null otherwise
  const [dragging, setDragging] = useState<number | null>(null);
  const scale = dragging ?? appearance.uiScale;

  switch (id) {
    case "theme":
      return (
        <Segmented
          label="Theme"
          value={appearance.theme}
          options={themes}
          onChange={(theme) => void updateSettings("appearance", { theme })}
        />
      );
    case "accent":
      return (
        <div className="flex items-center gap-1.5">
          {ACCENTS.map((accent) => (
            <button
              key={accent}
              type="button"
              aria-label={`Accent ${accent}`}
              aria-pressed={appearance.accent === accent}
              onClick={() => void updateSettings("appearance", { accent })}
              className={clsx(
                "v-focus size-6 rounded-full border-2 transition-transform duration-(--v-fast) hover:scale-110",
                appearance.accent === accent ? "border-ink" : "border-transparent",
              )}
              style={{ background: accent }}
            />
          ))}
          <label className="v-focus relative ml-1 size-6 cursor-pointer overflow-hidden rounded-full border border-line-strong bg-[conic-gradient(red,yellow,lime,cyan,blue,magenta,red)]">
            <span className="sr-only">Pick any accent colour</span>
            <input
              type="color"
              value={appearance.accent}
              onChange={(event) =>
                void updateSettings("appearance", { accent: event.target.value })
              }
              className="absolute inset-0 cursor-pointer opacity-0"
            />
          </label>
        </div>
      );
    case "ui-scale":
      return (
        <div className="flex items-center gap-3">
          <Slider
            label="Interface size"
            value={scale}
            min={0.75}
            max={2}
            step={0.05}
            onChange={(value) => {
              setDragging(value);
              // preview straight away, save once the slider is let go
              applyAppearance({ ...appearance, uiScale: value });
            }}
            onCommit={(uiScale) => {
              void updateSettings("appearance", { uiScale }).finally(() => {
                setDragging(null);
              });
            }}
          />
          <span className="w-10 text-right text-sm text-ink-soft tabular-nums">
            {Math.round(scale * 100)}%
          </span>
        </div>
      );
    case "compact":
      return (
        <Switch
          label="Compact mode"
          checked={appearance.compact}
          onChange={(compact) => void updateSettings("appearance", { compact })}
        />
      );
    case "reduced-motion":
      return (
        <Switch
          label="Reduce motion"
          checked={appearance.reducedMotion}
          onChange={(reducedMotion) => void updateSettings("appearance", { reducedMotion })}
        />
      );
    case "autosave":
      return (
        <Select
          label="Autosave"
          value={String(general.autosaveSeconds)}
          options={withValue(AUTOSAVE, general.autosaveSeconds)}
          onChange={(value) => void updateSettings("general", { autosaveSeconds: Number(value) })}
        />
      );
    case "recent-limit":
      return (
        <Select
          label="Recent files to remember"
          value={String(general.recentLimit)}
          options={withValue(RECENT, general.recentLimit)}
          onChange={(value) => void updateSettings("general", { recentLimit: Number(value) })}
        />
      );
    case "notify-jobs":
      return (
        <Switch
          label="Notify when jobs finish"
          checked={general.notifyJobs}
          onChange={(notifyJobs) => void updateSettings("general", { notifyJobs })}
        />
      );
    case "gpu":
      return (
        <Switch
          label="GPU acceleration"
          checked={performance.gpuAcceleration}
          onChange={(gpuAcceleration) => void updateSettings("performance", { gpuAcceleration })}
        />
      );
    case "cpu-jobs":
      return (
        <Select
          label="Jobs at the same time"
          value={String(performance.maxCpuJobs)}
          options={withValue(CPU_JOBS, performance.maxCpuJobs)}
          onChange={(value) => void updateSettings("performance", { maxCpuJobs: Number(value) })}
        />
      );
    case "gpu-jobs":
      return (
        <Select
          label="GPU jobs at the same time"
          value={String(performance.maxGpuJobs)}
          options={withValue(GPU_JOBS, performance.maxGpuJobs)}
          onChange={(value) => void updateSettings("performance", { maxGpuJobs: Number(value) })}
        />
      );
    case "low-memory":
      return (
        <Switch
          label="Low memory mode"
          checked={performance.lowMemoryMode}
          onChange={(lowMemoryMode) => void updateSettings("performance", { lowMemoryMode })}
        />
      );
    case "shortcut-preset":
      return (
        <Select
          label="Shortcut preset"
          value={shortcuts.preset}
          options={PRESET_ORDER.map((preset) => ({
            value: preset,
            label: PRESETS[preset].label,
          }))}
          onChange={(value) => {
            const preset = PRESET_ORDER.find((entry) => entry === value);
            if (preset) void updateSettings("shortcuts", { preset });
          }}
        />
      );
    default:
      return null;
  }
}

function SettingRow({
  entry,
  sections,
  flash,
}: {
  entry: SettingEntry;
  sections: SettingsMap;
  flash: boolean;
}) {
  const description =
    entry.id === "shortcut-preset"
      ? PRESETS[sections.shortcuts.preset].description
      : entry.description;
  return (
    <div
      id={`setting-${entry.id}`}
      className={clsx(
        "flex items-center gap-6 rounded-lg px-3 py-3 transition-colors duration-(--v-slow)",
        flash && "bg-accent-soft",
      )}
    >
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{entry.title}</p>
        <p className="mt-0.5 text-xs text-ink-soft">{description}</p>
      </div>
      <div className="shrink-0">
        <Control id={entry.id} sections={sections} />
      </div>
    </div>
  );
}

export function SettingsDialog() {
  const pane = useUi((state) => state.settingsPane);
  const focus = useUi((state) => state.settingsFocus);
  const openSettings = useUi((state) => state.openSettings);
  const close = useUi((state) => state.closeSettings);
  const sections = useSettings((state) => state.sections);
  const [query, setQuery] = useState("");
  // a setting picked in the palette is shown in its pane and flashes once
  const flash = focus;

  useEffect(() => {
    if (!focus || !pane) return;
    const frame = requestAnimationFrame(() => {
      document.getElementById(`setting-${focus}`)?.scrollIntoView({ block: "center" });
    });
    const timer = setTimeout(() => {
      useUi.setState({ settingsFocus: null });
    }, 1400);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  }, [focus, pane]);

  const matches = useMemo(
    () =>
      query.trim() && !focus
        ? fuzzyRank(
            query,
            SETTINGS.map((entry) => ({
              ...entry,
              label: entry.title,
              keywords: [...entry.keywords, entry.description],
            })),
          ).map((result) => result.item)
        : null,
    [query, focus],
  );

  const active = pane ?? "appearance";

  return (
    <Dialog
      open={pane !== null}
      onOpenChange={(open) => {
        if (!open) {
          close();
          setQuery("");
        }
      }}
      title="Settings"
      description="Change how Vellum looks, saves your work and uses your computer."
      className="top-[8vh] h-[min(42rem,84vh)] max-h-none w-[min(56rem,calc(100vw-2rem))]"
    >
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Settings sections"
          className="flex w-52 shrink-0 flex-col gap-0.5 border-r border-line p-3"
        >
          <label className="mb-2 flex h-8 items-center gap-2 rounded-md border border-line bg-canvas px-2.5 focus-within:border-accent">
            <Search className="size-3.5 shrink-0 text-ink-faint" aria-hidden />
            <input
              aria-label="Search settings"
              placeholder="Search settings"
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
              }}
              className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint"
            />
          </label>
          {PANES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              aria-current={!matches && active === entry.id ? "page" : undefined}
              onClick={() => {
                setQuery("");
                openSettings(entry.id);
              }}
              className={clsx(
                "v-focus flex h-8 items-center rounded-md px-2.5 text-left text-sm",
                !matches && active === entry.id
                  ? "bg-hover font-medium text-ink"
                  : "text-ink-soft hover:bg-hover hover:text-ink",
              )}
            >
              {entry.title}
            </button>
          ))}
        </nav>

        <div className="min-w-0 flex-1 overflow-y-auto p-3">
          {!sections ? (
            <p className="p-3 text-sm text-ink-soft">
              Settings load once Vellum is connected to its backend.
            </p>
          ) : matches ? (
            <SearchResults matches={matches} sections={sections} />
          ) : active === "shortcuts" ? (
            <ShortcutEditor
              presetRow={
                <SettingRow
                  entry={PRESET_SETTING}
                  sections={sections}
                  flash={flash === "shortcut-preset"}
                />
              }
            />
          ) : (
            <PaneRows pane={active} sections={sections} flash={flash} />
          )}
        </div>
      </div>
    </Dialog>
  );
}

function PaneRows({
  pane,
  sections,
  flash,
}: {
  pane: SettingsPane;
  sections: SettingsMap;
  flash: string | null;
}) {
  const title = PANES.find((entry) => entry.id === pane)?.title;
  return (
    <section aria-label={title}>
      <h2 className="px-3 pt-1 pb-2 text-lg font-semibold">{title}</h2>
      <div className="divide-y divide-line">
        {SETTINGS.filter((entry) => entry.pane === pane).map((entry) => (
          <SettingRow key={entry.id} entry={entry} sections={sections} flash={flash === entry.id} />
        ))}
      </div>
    </section>
  );
}

function SearchResults({ matches, sections }: { matches: SettingEntry[]; sections: SettingsMap }) {
  if (matches.length === 0) {
    return <p className="p-3 text-sm text-ink-soft">No setting matches that.</p>;
  }
  return (
    <section aria-label="Matching settings" className="divide-y divide-line">
      {matches.map((entry) => (
        <div key={entry.id}>
          <p className="px-3 pt-2 text-2xs font-medium tracking-wide text-ink-faint uppercase">
            {PANES.find((pane) => pane.id === entry.pane)?.title}
          </p>
          <SettingRow entry={entry} sections={sections} flash={false} />
        </div>
      ))}
    </section>
  );
}
