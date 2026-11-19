// editing shortcuts on top of a preset. the effective keymap is the preset plus the user's
// overrides, so every edit writes the whole chord list of the commands it touches.

type Bindings = Partial<Record<string, string[]>>;
type Overrides = Record<string, string[]>;

function copy(overrides: Bindings): Overrides {
  const out: Overrides = {};
  for (const [kind, chords] of Object.entries(overrides)) if (chords) out[kind] = [...chords];
  return out;
}

const workspaceContexts = ["quick_tools", "image", "video", "color", "audio", "motion", "vector"];

/** the same rule as shortcuts::context_of in rust */
export function contextOf(kind: string): string {
  const first = kind.split(".")[0] ?? "";
  return workspaceContexts.includes(first) ? first : "global";
}

function overlaps(a: string, b: string): boolean {
  return a === "global" || b === "global" || a === b;
}

/** the commands that already use a chord where it would clash with `kind` */
export function clashesFor(bindings: Bindings, kind: string, chord: string): string[] {
  return Object.entries(bindings)
    .filter(
      ([other, chords]) =>
        other !== kind && chords?.includes(chord) && overlaps(contextOf(kind), contextOf(other)),
    )
    .map(([other]) => other)
    .sort();
}

/** adds a chord to a command. with `take`, the commands that clash lose it. */
export function addChord(
  bindings: Bindings,
  overrides: Bindings,
  kind: string,
  chord: string,
  take: boolean,
): Overrides {
  const next = copy(overrides);
  const mine = bindings[kind] ?? [];
  if (!mine.includes(chord)) next[kind] = [...mine, chord];
  if (take) {
    for (const other of clashesFor(bindings, kind, chord)) {
      next[other] = (bindings[other] ?? []).filter((c) => c !== chord);
    }
  }
  return next;
}

export function removeChord(
  bindings: Bindings,
  overrides: Bindings,
  kind: string,
  chord: string,
): Overrides {
  return { ...copy(overrides), [kind]: (bindings[kind] ?? []).filter((c) => c !== chord) };
}

/** puts one command back to what the preset says */
export function resetCommand(overrides: Bindings, kind: string): Overrides {
  const next = copy(overrides);
  // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
  delete next[kind];
  return next;
}
