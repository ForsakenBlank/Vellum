import { describe, expect, it } from "vitest";

import { useApp } from "../stores/app";
import { useJobs } from "../stores/jobs";
import { useSettings } from "../stores/settings";
import { appInfo, job, readyBackend, settings, connectTo } from "../test/fakes";

describe("connect", () => {
  it("loads state, follows events and reports ready last", async () => {
    const backend = readyBackend().reply("jobs_overview", () => ({
      jobs: [job({ id: 1 })],
      pending: [],
    }));
    await connectTo(backend);

    expect(useApp.getState()).toMatchObject({ status: "ready", info: appInfo });
    expect(useSettings.getState().sections).toEqual(settings);
    expect(useJobs.getState().jobs[1]?.name).toBe("Export clip.mp4");
    expect(backend.calls.at(-1)?.command).toBe("ui_ready");

    backend.emit("jobs", { type: "updated", job: job({ id: 2, name: "Second" }) });
    expect(useJobs.getState().jobs[2]?.name).toBe("Second");
  });

  it("applies the theme whenever appearance settings change", async () => {
    const backend = readyBackend();
    await connectTo(backend);
    expect(document.documentElement.dataset.theme).toBe("dark");

    backend.emit("settings.changed", {
      section: "appearance",
      value: { ...settings.appearance, theme: "light" },
    });
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("stops following events after disconnect", async () => {
    const backend = readyBackend();
    const disconnect = await connectTo(backend);
    disconnect();

    backend.emit("jobs", { type: "updated", job: job({ id: 9 }) });
    expect(useJobs.getState().jobs[9]).toBeUndefined();
    expect(backend.listenerCount).toBe(0);
    expect(useApp.getState().api).toBeNull();
  });

  it("says so when there is no desktop backend", async () => {
    await connectTo(null);
    expect(useApp.getState().status).toBe("browser");
  });

  it("shows why startup failed", async () => {
    const backend = readyBackend().fail("settings_all", {
      code: "settings.io",
      message: "Your settings could not be read.",
    });
    await connectTo(backend);

    expect(useApp.getState()).toMatchObject({
      status: "failed",
      notice: "Your settings could not be read.",
    });
    expect(backend.callsTo("ui_ready")).toHaveLength(0);
  });
});
