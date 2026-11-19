// keyboard chords, in the same canonical form rust uses: `Ctrl+Alt+Shift+Meta+Key`.
// keys are physical keys named after the us layout (event.code), so shortcuts stay in place
// on other layouts. on macos the command key counts as Ctrl, which is what mac users expect
// from Ctrl+Z style shortcuts in cross platform apps.

/** the bits of a KeyboardEvent the chord logic needs, so tests can pass plain objects */
export interface KeyLike {
  code: string;
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export const isMac =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || "");

const named: Record<string, string> = {
  Enter: "Enter",
  NumpadEnter: "Enter",
  Escape: "Escape",
  Tab: "Tab",
  Space: "Space",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  NumpadAdd: "=",
  NumpadSubtract: "-",
};

/** the key part of a chord, or null for modifier keys and keys we do not bind */
export function keyFromCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter?.[1]) return letter[1];
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(code);
  if (digit?.[1]) return digit[1];
  const fn = /^F([0-9]{1,2})$/.exec(code);
  if (fn?.[1] && Number(fn[1]) >= 1 && Number(fn[1]) <= 24) return `F${fn[1]}`;
  return named[code] ?? null;
}

/** turns a key press into a chord, or null if it was only a modifier */
export function chordFromEvent(event: KeyLike, mac = isMac): string | null {
  const key = keyFromCode(event.code);
  if (!key) return null;
  const ctrl = event.ctrlKey || (mac && event.metaKey);
  const meta = !mac && event.metaKey;
  const parts: string[] = [];
  if (ctrl) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (meta) parts.push("Meta");
  parts.push(key);
  return parts.join("+");
}

const macSymbols: Record<string, string> = {
  Ctrl: "⌘",
  Alt: "⌥",
  Shift: "⇧",
  Meta: "⌃",
  Enter: "↩",
  Escape: "Esc",
  Backspace: "⌫",
  Delete: "⌦",
  Up: "↑",
  Down: "↓",
  Left: "←",
  Right: "→",
  Tab: "⇥",
};

const pcNames: Record<string, string> = {
  Meta: "Win",
  Up: "↑",
  Down: "↓",
  Left: "←",
  Right: "→",
  Escape: "Esc",
};

/** splits a chord into the labels shown in a key cap, eg ["Ctrl", "Shift", "Z"] */
export function chordParts(chord: string, mac = isMac): string[] {
  // "+" is never a key in a chord (rust uses "=" for it), so splitting on it is safe
  return chord
    .split("+")
    .map((part) => (mac ? (macSymbols[part] ?? part) : (pcNames[part] ?? part)));
}

export function formatChord(chord: string, mac = isMac): string {
  return chordParts(chord, mac).join(mac ? "" : "+");
}

const textEditing = new Set([
  "Ctrl+Z",
  "Ctrl+Shift+Z",
  "Ctrl+Y",
  "Ctrl+A",
  "Ctrl+C",
  "Ctrl+V",
  "Ctrl+X",
  "Ctrl+Left",
  "Ctrl+Right",
  "Ctrl+Shift+Left",
  "Ctrl+Shift+Right",
  "Ctrl+Backspace",
  "Ctrl+Delete",
  "Ctrl+Home",
  "Ctrl+End",
]);

/**
 * true if a chord should reach the app even while typing in a text field. plain keys and
 * the usual editing chords stay with the field, so Ctrl+Z in the notes box undoes typing
 * and not the document.
 */
export function firesWhileTyping(chord: string): boolean {
  if (textEditing.has(chord)) return false;
  if (/^F[0-9]+$/.test(chord)) return true;
  return chord.startsWith("Ctrl+") || chord.startsWith("Alt+") || chord.includes("Meta+");
}

export function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "range", "color", "submit"].includes(target.type);
  }
  return false;
}

/** the command kinds bound to a chord, from a keymap's bindings */
export function commandsForChord(
  bindings: Partial<Record<string, string[]>>,
  chord: string,
): string[] {
  return Object.entries(bindings)
    .filter(([, chords]) => chords?.includes(chord))
    .map(([kind]) => kind);
}
