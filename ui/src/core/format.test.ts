import { describe, expect, it } from "vitest";
import { formatEta, formatPercent } from "./format";

describe("formatEta", () => {
  it("picks a sensible unit", () => {
    expect(formatEta(4.4)).toBe("4s");
    expect(formatEta(125)).toBe("2m 05s");
    expect(formatEta(3720)).toBe("1h 02m");
  });

  it("returns null when there is no estimate", () => {
    expect(formatEta(null)).toBeNull();
    expect(formatEta(Number.POSITIVE_INFINITY)).toBeNull();
    expect(formatEta(-1)).toBeNull();
  });
});

describe("formatPercent", () => {
  it("clamps and floors", () => {
    expect(formatPercent(0.999)).toBe("99%");
    expect(formatPercent(2)).toBe("100%");
    expect(formatPercent(-1)).toBe("0%");
  });
});
