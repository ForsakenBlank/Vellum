import { describe, expect, it } from "vitest";

import { accentSwatches, contrastRatio, inkFor, isHexColour, luminance } from "./color";

describe("colour helpers", () => {
  it("only accepts #rrggbb", () => {
    expect(isHexColour("#f2a93b")).toBe(true);
    expect(isHexColour("#fff")).toBe(false);
    expect(isHexColour("red")).toBe(false);
  });

  it("measures luminance and contrast like wcag", () => {
    expect(luminance("#000000")).toBe(0);
    expect(luminance("#ffffff")).toBeCloseTo(1);
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21);
  });

  it("picks dark text on light accents and white on dark ones", () => {
    expect(inkFor("#f2a93b")).toBe("#16120b");
    expect(inkFor("#1a237e")).toBe("#ffffff");
    expect(inkFor("nonsense")).toBe("#16120b");
  });

  it("every swatch keeps its text readable", () => {
    for (const swatch of accentSwatches) {
      expect(contrastRatio(swatch.value, inkFor(swatch.value))).toBeGreaterThanOrEqual(4.5);
    }
  });
});
