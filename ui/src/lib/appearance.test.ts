import { describe, expect, it } from "vitest";

import { settings } from "../test/fakes";
import { applyAppearance } from "./appearance";

describe("applyAppearance", () => {
  it("puts theme, accent, scale and motion on the root element", () => {
    const root = document.createElement("html");
    applyAppearance(
      { ...settings.appearance, theme: "highContrast", accent: "#ff8800", uiScale: 1.25 },
      root,
    );

    expect(root.dataset.theme).toBe("highContrast");
    expect(root.style.getPropertyValue("--v-accent")).toBe("#ff8800");
    expect(root.style.fontSize).toBe("20px");
    expect(root.dataset.reducedMotion).toBe("false");
  });
});
