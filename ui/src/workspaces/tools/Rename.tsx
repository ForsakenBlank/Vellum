// batch rename inside the converter. the pattern is part of the session's options, so it
// undoes and autosaves like any other option. rust works out the new names (it can see
// which names are taken) and the list shows them before anything happens. renaming is a
// document command, so ctrl+z puts the old names back.

import { ArrowRight } from "lucide-react";

import type { NameCase } from "../../bindings/NameCase";
import type { RenamePattern } from "../../bindings/RenamePattern";
import type { RenamePlan } from "../../bindings/RenamePlan";
import { Button } from "../../components/Button";
import { fileName } from "../../lib/routing";
import { Field, Note, NumberField, Section, Segmented, TextField } from "./parts";

const tags: { tag: string; tooltip: string }[] = [
  { tag: "{name}", tooltip: "The old name, after find and replace" },
  { tag: "{n}", tooltip: "A number that counts up down the list" },
  { tag: "{date}", tooltip: "The day the file was last changed, like 2026-11-21" },
  { tag: "{folder}", tooltip: "The name of the folder the file is in" },
  { tag: "{ext}", tooltip: "The old extension, without the dot" },
];

export function RenameOptions({
  pattern,
  set,
  seal,
  plan,
}: {
  pattern: RenamePattern;
  set: (patch: Partial<RenamePattern>) => void;
  seal: () => void;
  plan: RenamePlan | null;
}) {
  const usesNumber = pattern.pattern.includes("{n}");
  return (
    <>
      <Section title="New names">
        <TextField
          label="Pattern"
          value={pattern.pattern}
          placeholder="holiday {n}"
          onChange={(value) => {
            set({ pattern: value });
          }}
        />
        <div className="flex flex-wrap gap-1.5" aria-label="Tags">
          {tags.map(({ tag, tooltip }) => (
            <Button
              key={tag}
              size="sm"
              variant="secondary"
              tooltip={`${tooltip}. Click to add it.`}
              onClick={() => {
                set({ pattern: `${pattern.pattern}${tag}` });
                seal();
              }}
            >
              {tag}
            </Button>
          ))}
        </div>
        {plan?.error && <Note tone="danger">{plan.error}</Note>}
        <p className="text-2xs text-fg-subtle">
          Files keep their extension and stay in their folder. Names that are already taken are
          skipped, so nothing is ever replaced.
        </p>
      </Section>
      <Section title="Options">
        {usesNumber && (
          <div className="flex flex-wrap gap-4">
            <NumberField
              label="Start counting at"
              min={0}
              value={pattern.start}
              onChange={(start) => {
                if (start !== null) set({ start: Math.max(0, Math.round(start)) });
              }}
            />
            <NumberField
              label="Digits"
              min={1}
              max={9}
              value={pattern.digits}
              hint="2 gives 01, 02, 03."
              onChange={(digits) => {
                if (digits !== null) set({ digits: Math.min(9, Math.max(1, Math.round(digits))) });
              }}
            />
          </div>
        )}
        <div className="flex flex-wrap gap-4">
          <TextField
            label="Find"
            value={pattern.find}
            placeholder="IMG_"
            onChange={(find) => {
              set({ find });
            }}
          />
          <TextField
            label="Replace with"
            value={pattern.replace}
            placeholder="nothing"
            onChange={(replace) => {
              set({ replace });
            }}
          />
        </div>
        <Field label="Letters">
          <Segmented<NameCase>
            label="Letters"
            value={pattern.case}
            choices={[
              { value: "keep", label: "As typed", tooltip: "Keep capitals as they are" },
              { value: "lower", label: "lowercase", tooltip: "Every letter small" },
              { value: "upper", label: "UPPERCASE", tooltip: "Every letter a capital" },
            ]}
            onChange={(value) => {
              set({ case: value });
            }}
          />
        </Field>
      </Section>
    </>
  );
}

/** the old and new name of every file, with the reason a file will be skipped */
export function RenamePreview({ plan, loading }: { plan: RenamePlan | null; loading: boolean }) {
  if (!plan) {
    return <p className="text-xs text-fg-subtle">{loading ? "Working out the names…" : ""}</p>;
  }
  return (
    <Section title="Preview">
      <ul aria-label="New names" className="flex flex-col divide-y divide-line text-xs">
        {plan.rows.map((row) => {
          const same = row.from === row.to;
          return (
            <li key={row.from} className="flex flex-col gap-0.5 py-1.5">
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate text-fg-muted" title={row.from}>
                  {fileName(row.from)}
                </span>
                <ArrowRight className="size-3 shrink-0 text-fg-subtle" aria-label="becomes" />
                <span
                  className={`truncate font-medium ${
                    row.problem
                      ? "text-fg-subtle line-through"
                      : same
                        ? "text-fg-subtle"
                        : "text-fg"
                  }`}
                  title={row.to}
                >
                  {same && !row.problem ? "no change" : fileName(row.to)}
                </span>
              </div>
              {row.problem && <span className="text-2xs text-warning">{row.problem}</span>}
            </li>
          );
        })}
      </ul>
    </Section>
  );
}
