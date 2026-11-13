/**
 * keyboard shortcuts as plain strings, such as "Mod+Shift+K".
 *
 * "Mod" is ctrl on windows and linux and cmd on a mac, so one string works on
 * every platform. modifiers always come in the same order, which makes two
 * strings for the same keys compare equal.
 */

const MODIFIERS = ["Mod", "Ctrl", "Alt", "Shift"] as const;
type Modifier = (typeof MODIFIERS)[number];

export function isMac(): boolean {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
}

// keys named by event.code, so they do not change with shift or the keyboard layout
const CODE_KEYS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  Semicolon: ";",
  Quote: "'",
  Space: "Space",
  NumpadAdd: "=",
  NumpadSubtract: "-",
};

const NAMED_KEYS: Record<string, string> = {
  " ": "Space",
  Esc: "Escape",
  Del: "Delete",
  Left: "ArrowLeft",
  Right: "ArrowRight",
  Up: "ArrowUp",
  Down: "ArrowDown",
};

/** the key part of a shortcut, or null for a key that only modifies */
function keyName(event: Pick<KeyboardEvent, "key" | "code">): string | null {
  const { code, key } = event;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^Numpad\d$/.test(code)) return code.slice(6);
  const byCode = CODE_KEYS[code];
  if (byCode) return byCode;
  if (["Control", "Shift", "Alt", "Meta", "OS", "Dead", "Unidentified", "Process"].includes(key)) {
    return null;
  }
  const named = NAMED_KEYS[key] ?? key;
  return named.length === 1 ? named.toUpperCase() : named;
}

/** the shortcut a key press makes, or null while only modifiers are held */
export function shortcutFromEvent(
  event: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
  mac: boolean = isMac(),
): string | null {
  const key = keyName(event);
  if (!key) return null;
  const parts: Modifier[] = [];
  if (mac ? event.metaKey : event.ctrlKey) parts.push("Mod");
  if (mac && event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  return [...parts, key].join("+");
}

/** puts modifiers in the standard order, so equal shortcuts are equal strings */
export function normalizeShortcut(shortcut: string): string {
  const parts = shortcut.split("+").filter(Boolean);
  // "Mod++" means mod and the plus key
  if (shortcut.endsWith("++")) parts.push("+");
  const key = parts.pop() ?? "";
  const mods = MODIFIERS.filter((mod) =>
    parts.some((part) => part.toLowerCase() === mod.toLowerCase()),
  );
  const named = key.length === 1 ? key.toUpperCase() : key;
  return [...mods, named].join("+");
}

/** true for shortcuts that are just a key, which must not fire while typing */
export function isBareKey(shortcut: string): boolean {
  const parts = normalizeShortcut(shortcut).split("+");
  return parts.length === 1 || (parts.length === 2 && parts[0] === "Shift");
}

const MAC_SYMBOLS: Record<string, string> = {
  Mod: "⌘",
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
};

const KEY_LABELS: Record<string, string> = {
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Escape: "Esc",
  Delete: "Del",
  Backspace: "⌫",
  Enter: "Enter",
};

/** how a shortcut is shown to people: "Ctrl+Shift+K" or "⇧⌘K" on a mac */
export function formatShortcut(shortcut: string, mac: boolean = isMac()): string {
  const parts = normalizeShortcut(shortcut).split("+");
  const key = parts.pop() ?? "";
  const label = KEY_LABELS[key] ?? key;
  if (mac) {
    // mac menus list ctrl, alt, shift, cmd in that order
    const order = ["Ctrl", "Alt", "Shift", "Mod"];
    const mods = order.filter((mod) => parts.includes(mod)).map((mod) => MAC_SYMBOLS[mod] ?? mod);
    return [...mods, label].join("");
  }
  return [...parts.map((mod) => (mod === "Mod" ? "Ctrl" : mod)), label].join("+");
}

/** true when the key press came from somewhere the user is typing */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "range", "button", "color"].includes(target.type);
  }
  return false;
}
