import { describe, expect, it } from "vitest";

import { formatDuration, formatPercent } from "./format";

describe("formatDuration", () => {
  it("uses the largest sensible unit", () => {
    expect(formatDuration(0)).toBe("0 s");
    expect(formatDuration(44.6)).toBe("45 s");
    expect(formatDuration(60)).toBe("1 min");
    expect(formatDuration(200)).toBe("3 min 20 s");
    expect(formatDuration(7200)).toBe("2 h");
    expect(formatDuration(7500)).toBe("2 h 5 min");
  });

  it("never shows negative time", () => {
    expect(formatDuration(-5)).toBe("0 s");
  });
});

describe("formatPercent", () => {
  it("rounds down and clamps", () => {
    expect(formatPercent(0.256)).toBe("25%");
    expect(formatPercent(0.999)).toBe("99%");
    expect(formatPercent(1)).toBe("100%");
    expect(formatPercent(3)).toBe("100%");
    expect(formatPercent(-1)).toBe("0%");
  });
});
