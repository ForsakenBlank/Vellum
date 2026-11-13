import { describe, expect, it } from "vitest";

import { createApi } from "../lib/api";
import { FakeBackend, settings } from "../test/fakes";
import { useApp } from "./app";
import { updateSettings, useSettings } from "./settings";

function connected(backend: FakeBackend) {
  useApp.setState({ api: createApi(backend), status: "ready" });
  useSettings.getState().applyAll(settings);
}

describe("settings store", () => {
  it("sends the whole section and keeps what the backend stored", async () => {
    const backend = new FakeBackend().reply("settings_set", ({ value }) => ({
      ...(value as object),
      // the backend clamps values it does not accept as they are
      uiScale: 2,
    }));
    connected(backend);

    await updateSettings("appearance", { uiScale: 9 });

    expect(backend.callsTo("settings_set")).toEqual([
      { section: "appearance", value: { ...settings.appearance, uiScale: 9 } },
    ]);
    expect(useSettings.getState().sections?.appearance.uiScale).toBe(2);
  });

  it("follows changes made elsewhere", () => {
    useSettings.getState().applyAll(settings);
    useSettings.getState().applyChange({
      section: "performance",
      value: { ...settings.performance, lowMemoryMode: true },
    });
    expect(useSettings.getState().sections?.performance.lowMemoryMode).toBe(true);
    expect(useSettings.getState().sections?.appearance).toEqual(settings.appearance);
  });

  it("keeps the old value and tells the user when saving fails", async () => {
    const backend = new FakeBackend().fail("settings_set", {
      code: "settings.io",
      message: "Your settings could not be saved.",
    });
    connected(backend);

    await updateSettings("appearance", { theme: "light" });

    expect(useSettings.getState().sections?.appearance.theme).toBe("dark");
    expect(useApp.getState().notice).toBe("Your settings could not be saved.");
  });
});
