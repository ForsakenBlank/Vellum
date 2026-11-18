import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppSettings } from "../bindings/AppSettings";
import { TooltipProvider } from "../components/Tooltip";
import { ipc } from "../lib/ipc";
import { applyAppearance, useSettings } from "../state/settings";
import { AppShell } from "./AppShell";

const defaults: AppSettings = {
  appearance: { theme: "dark", accent: "#7c5cff", uiScale: 1, reducedMotion: false },
  autosave: { enabled: true, intervalSecs: 60 },
  jobs: { maxCpuJobs: 2, maxGpuJobs: 1, maxIoJobs: 4 },
  history: { maxSteps: 500, maxMemoryMb: 1024 },
};

vi.mock("../lib/ipc", () => ({
  inDesktopApp: false,
  ipc: { updateSettings: vi.fn() },
}));

function renderShell() {
  render(
    <TooltipProvider>
      <AppShell />
    </TooltipProvider>,
  );
}

describe("AppShell", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useSettings.setState({ settings: defaults, error: null });
  });

  it("says when it is not connected to the desktop app", () => {
    renderShell();
    expect(screen.getByRole("status")).toHaveTextContent("running in a browser");
  });

  it("changes the theme through rust and follows what rust sends back", async () => {
    const user = userEvent.setup();
    const light = { ...defaults, appearance: { ...defaults.appearance, theme: "light" as const } };
    vi.mocked(ipc.updateSettings).mockResolvedValueOnce(light);
    renderShell();

    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(ipc.updateSettings).toHaveBeenCalledWith({ appearance: { theme: "light" } });
    expect(useSettings.getState().settings?.appearance.theme).toBe("light");
  });

  it("shows a helpful message when saving settings fails", async () => {
    const user = userEvent.setup();
    vi.mocked(ipc.updateSettings).mockRejectedValueOnce("disk is full");
    renderShell();
    await user.click(screen.getByRole("radio", { name: "High contrast" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Settings could not be saved: disk is full",
    );
  });

  it("lists every workspace as coming later", () => {
    renderShell();
    const nav = screen.getByRole("navigation", { name: "Workspaces" });
    expect(nav.querySelectorAll("button")).toHaveLength(7);
    expect(screen.getByRole("button", { name: "Image" })).toBeDisabled();
  });
});

describe("applyAppearance", () => {
  it("puts the theme, accent, scale and motion preference on the root element", () => {
    const root = document.createElement("html");
    applyAppearance(
      {
        ...defaults,
        appearance: {
          theme: "high-contrast",
          accent: "#ff0000",
          uiScale: 1.25,
          reducedMotion: true,
        },
      },
      root,
    );
    expect(root.dataset.theme).toBe("high-contrast");
    expect(root.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(root.style.fontSize).toBe("20px");
    expect(root.dataset.reducedMotion).toBe("");

    applyAppearance(defaults, root);
    expect(root.dataset.reducedMotion).toBeUndefined();
  });
});
