import { describe, expect, it } from "vitest";

import { formatAgo, formatBytes, formatDuration, formatPercent } from "./format";

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

describe("formatBytes", () => {
  it("picks a readable unit", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(10_000_000)).toBe("10.0 MB");
    expect(formatBytes(250_000_000)).toBe("250 MB");
  });
});

describe("formatAgo", () => {
  it("rounds to the unit people say out loud", () => {
    const now = 1_000_000_000;
    expect(formatAgo(now - 10_000, now)).toBe("just now");
    expect(formatAgo(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(formatAgo(now - 3 * 3_600_000, now)).toBe("3 h ago");
    expect(formatAgo(now - 30 * 3_600_000, now)).toBe("yesterday");
    expect(formatAgo(now - 72 * 3_600_000, now)).toBe("3 days ago");
  });
});
