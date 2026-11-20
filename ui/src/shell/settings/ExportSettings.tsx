// the export section of settings: the size export to size starts at, the safety margin,
// where results go, and the platform presets, which people edit when a platform changes
// its limit.

import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";

import type { ExportSettings } from "../../bindings/ExportSettings";
import type { SizePreset } from "../../bindings/SizePreset";
import type { SizeUnit } from "../../bindings/SizeUnit";
import { Button, IconButton } from "../../components/Button";
import { inDesktopApp } from "../../lib/ipc";
import { toolDefaults } from "../../lib/tools";
import { useSettings } from "../../state/settings";

const defaultPresets = (toolDefaults as unknown as { sizePresets: SizePreset[] }).sizePresets;

function useExport(): [ExportSettings | null, (patch: Partial<ExportSettings>) => void] {
  const settings = useSettings((s) => s.settings?.export ?? null);
  const update = useSettings((s) => s.update);
  return [settings, (patch) => void update({ export: patch })];
}

/** a number that may have decimals, sent when the field loses focus or enter is pressed */
function Amount({
  id,
  value,
  min,
  max,
  label,
  onChange,
}: {
  id?: string;
  value: number;
  min: number;
  max: number;
  label: string;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(String(value));
  }
  const commit = () => {
    const n = Math.round(Number(draft) * 100) / 100;
    if (Number.isFinite(n) && n !== value) onChange(Math.min(max, Math.max(min, n)));
    else setDraft(String(value));
  };
  return (
    <input
      id={id}
      aria-label={label}
      type="number"
      inputMode="decimal"
      step={0.5}
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
  );
}

function UnitPicker({ value, onChange }: { value: SizeUnit; onChange: (unit: SizeUnit) => void }) {
  return (
    <select
      aria-label="Unit"
      value={value}
      onChange={(event) => {
        onChange(event.target.value as SizeUnit);
      }}
      className="h-(--row-sm) rounded-md border border-line-strong bg-raised px-1.5 text-xs text-fg outline-none focus:border-accent-line"
    >
      <option value="MB">MB</option>
      <option value="MiB">MiB</option>
    </select>
  );
}

export function TargetControl({ id }: { id: string }) {
  const [settings, patch] = useExport();
  if (!settings) return null;
  return (
    <span className="flex items-center gap-2">
      <Amount
        id={id}
        label="Starting size"
        value={settings.targetAmount}
        min={0.01}
        max={100000}
        onChange={(targetAmount) => {
          patch({ targetAmount });
        }}
      />
      <UnitPicker
        value={settings.unit}
        onChange={(unit) => {
          patch({ unit });
        }}
      />
    </span>
  );
}

export function MarginControl({ id }: { id: string }) {
  const [settings, patch] = useExport();
  if (!settings) return null;
  return (
    <span className="flex items-center gap-2 text-xs text-fg-muted">
      <Amount
        id={id}
        label="Safety margin"
        value={settings.safetyMarginPercent}
        min={0}
        max={50}
        onChange={(safetyMarginPercent) => {
          patch({ safetyMarginPercent });
        }}
      />
      %
    </span>
  );
}

export function FolderControl() {
  const [settings, patch] = useExport();
  if (!settings) return null;
  const folder = settings.outputFolder;
  return (
    <span className="flex max-w-80 items-center gap-2">
      <span className="min-w-0 truncate text-xs text-fg-muted" title={folder ?? undefined}>
        {folder ?? "Next to the original"}
      </span>
      <Button
        size="sm"
        tooltip="Pick a folder for everything the quick tools make"
        icon={<FolderOpen className="size-3.5" />}
        disabled={!inDesktopApp}
        onClick={() => {
          void open({ directory: true, title: "Save results in" }).then((picked) => {
            if (typeof picked === "string") patch({ outputFolder: picked });
          });
        }}
      >
        Choose
      </Button>
      {folder !== null && (
        <Button
          size="sm"
          variant="ghost"
          tooltip="Save each result next to the file it came from"
          onClick={() => {
            patch({ outputFolder: null });
          }}
        >
          Reset
        </Button>
      )}
    </span>
  );
}

/** the preset chips, each with a name, a size and an optional note */
export function PresetsEditor() {
  const [settings, patch] = useExport();
  if (!settings) return null;
  const presets = settings.presets;
  const save = (next: SizePreset[]) => {
    patch({ presets: next });
  };
  const change = (index: number, edit: Partial<SizePreset>) => {
    save(presets.map((p, i) => (i === index ? { ...p, ...edit } : p)));
  };
  return (
    <div className="flex flex-col gap-2">
      <ul className="flex flex-col gap-1.5" aria-label="Size presets">
        {presets.map((preset, index) => (
          <li key={preset.id} className="flex items-center gap-2">
            <PresetLabel
              value={preset.label}
              onChange={(label) => {
                change(index, { label });
              }}
            />
            <Amount
              label={`Size of ${preset.label}`}
              value={preset.amount}
              min={0.01}
              max={100000}
              onChange={(amount) => {
                change(index, { amount });
              }}
            />
            <UnitPicker
              value={preset.unit}
              onChange={(unit) => {
                change(index, { unit });
              }}
            />
            <span className="min-w-0 flex-1 truncate text-2xs text-fg-subtle" title={preset.note}>
              {preset.note}
            </span>
            <IconButton
              size="sm"
              label={`Remove ${preset.label}`}
              icon={<Trash2 className="size-3.5" />}
              onClick={() => {
                save(presets.filter((_, i) => i !== index));
              }}
            />
          </li>
        ))}
      </ul>
      <div className="flex gap-2">
        <Button
          size="sm"
          tooltip="Add a size of your own"
          icon={<Plus className="size-3.5" />}
          disabled={presets.length >= 32}
          onClick={() => {
            save([
              ...presets,
              {
                id: `custom-${String(Date.now())}`,
                label: "My limit",
                amount: 25,
                unit: "MB",
                note: "",
              },
            ]);
          }}
        >
          Add preset
        </Button>
        <Button
          size="sm"
          variant="ghost"
          tooltip="Put back the limits Vellum shipped with"
          icon={<RotateCcw className="size-3.5" />}
          onClick={() => {
            save(defaultPresets);
          }}
        >
          Reset to defaults
        </Button>
      </div>
    </div>
  );
}

function PresetLabel({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }
  const commit = () => {
    const next = draft.trim();
    if (next && next !== value) onChange(next);
    else setDraft(value);
  };
  return (
    <input
      aria-label="Preset name"
      value={draft}
      maxLength={40}
      onChange={(event) => {
        setDraft(event.target.value);
      }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
      }}
      className="h-(--row-sm) w-44 rounded-md border border-line bg-raised px-2 text-sm text-fg outline-none focus:border-accent-line"
    />
  );
}
