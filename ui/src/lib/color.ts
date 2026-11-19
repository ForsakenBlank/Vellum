// small colour helpers for the accent setting

/** true for `#rrggbb`, the only format rust accepts */
export function isHexColour(text: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(text);
}

/** wcag relative luminance of a `#rrggbb` colour, 0 for black to 1 for white */
export function luminance(hex: string): number {
  const channel = (offset: number) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

const darkInk = "#16120b";
const lightInk = "#ffffff";

/** the text colour that reads best on top of an accent fill */
export function inkFor(accent: string): string {
  if (!isHexColour(accent)) return darkInk;
  return contrastRatio(accent, darkInk) >= contrastRatio(accent, lightInk) ? darkInk : lightInk;
}

/** accents offered as swatches in settings, picked to read well on every theme */
export const accentSwatches = [
  { name: "Safelight", value: "#f2a93b" },
  { name: "Ember", value: "#f2704a" },
  { name: "Rose", value: "#ec5f94" },
  { name: "Iris", value: "#8b7bff" },
  { name: "Glacier", value: "#4fa8ff" },
  { name: "Lagoon", value: "#2fc4b2" },
  { name: "Moss", value: "#7cc560" },
  { name: "Graphite", value: "#a3a3a3" },
];
