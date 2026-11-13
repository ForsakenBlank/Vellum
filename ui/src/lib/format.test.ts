import { describe, expect, it } from "vitest";

import { formatDuration, formatEta, formatPercent } from "./format";

describe("formatDuration", () => {
  it("uses the largest two units that matter", () => {
    expect(formatDuration(0)).toBe("0 s");
    expect(formatDuration(8_400)).toBe("8 s");
    expect(formatDuration(60_000)).toBe("1 min");
    expect(formatDuration(125_000)).toBe("2 min 5 s");
    expect(formatDuration(3_600_000)).toBe("1 h");
    expect(formatDuration(4_350_000)).toBe("1 h 12 min");
  });

  it("never goes negative", () => {
    expect(formatDuration(-500)).toBe("0 s");
  });
});

describe("formatEta", () => {
  it("reads as a sentence fragment", () => {
    expect(formatEta(12_000)).toBe("about 12 s left");
    expect(formatEta(400)).toBe("almost done");
  });
});

describe("formatPercent", () => {
  it("rounds and clamps", () => {
    expect(formatPercent(0.424)).toBe("42%");
    expect(formatPercent(1.7)).toBe("100%");
    expect(formatPercent(-1)).toBe("0%");
  });
});
