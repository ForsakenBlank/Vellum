import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import fixture from "../../e2e/fixtures/backend.json";
import type { AppSettings } from "../bindings/AppSettings";
import type { Keymap } from "../bindings/Keymap";
import { TooltipProvider } from "../components/Tooltip";
import { ipc } from "../lib/ipc";
import { useCommands } from "../state/commands";
import { useDocuments } from "../state/documents";
import { useRecent } from "../state/recent";
import { applyAppearance, useSettings } from "../state/settings";
import { useUi } from "../state/ui";
import { AppShell } from "./AppShell";

const defaults = fixture.settings as AppSettings;

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
    useCommands.setState({
      infos: fixture.commands,
      keymap: fixture.keymaps.vellum as Keymap,
    });
    useRecent.setState({ files: [] });
    useDocuments.setState({ documents: [], activeId: null, emptyWorkspace: null });
    useUi.setState({ settingsSection: null, paletteOpen: false, toasts: [] });
  });

  it("says when it is not connected to the desktop app", () => {
    renderShell();
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("running in a browser");
  });

  it("shows home with the quick tools and an empty recent list", () => {
    renderShell();
    expect(screen.getByRole("heading", { name: "Drop anything to start" })).toBeInTheDocument();
    for (const title of [
      "Export to 10 MB",
      "Video to GIF",
      "Remove background",
      "Convert",
      "Screen record",
    ]) {
      expect(screen.getByRole("button", { name: new RegExp(title) })).toBeInTheDocument();
    }
    expect(screen.getByText("No recent files yet")).toBeInTheDocument();
  });

  it("shows loading skeletons until recent files arrive", () => {
    useRecent.setState({ files: null });
    renderShell();
    expect(screen.getByLabelText("Loading recent files")).toHaveAttribute("aria-busy");
  });

  it("switches to an empty workspace from the rail", async () => {
    const user = userEvent.setup();
    renderShell();
    const rail = screen.getByRole("navigation", { name: "Workspaces" });
    await user.click(within(rail).getByRole("button", { name: "Audio" }));
    expect(screen.getByText("Audio workspace")).toBeInTheDocument();
    expect(within(rail).getByRole("button", { name: "Audio" })).toHaveAttribute(
      "aria-current",
      "true",
    );
  });

  it("changes the theme through rust from settings and follows what rust sends back", async () => {
    const user = userEvent.setup();
    const light = { ...defaults, appearance: { ...defaults.appearance, theme: "light" as const } };
    vi.mocked(ipc.updateSettings).mockResolvedValueOnce(light);
    useUi.getState().openSettings("appearance");
    renderShell();

    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(ipc.updateSettings).toHaveBeenCalledWith({ appearance: { theme: "light" } });
    expect(useSettings.getState().settings?.appearance.theme).toBe("light");
  });

  it("finds settings by search", async () => {
    const user = userEvent.setup();
    useUi.getState().openSettings("appearance");
    renderShell();
    await user.type(screen.getByRole("textbox", { name: "Search settings" }), "density");
    expect(screen.getByText("Compact mode")).toBeInTheDocument();
    expect(screen.queryByText("Accent colour")).not.toBeInTheDocument();
  });

  it("shows a helpful message when saving settings fails", async () => {
    const user = userEvent.setup();
    vi.mocked(ipc.updateSettings).mockRejectedValueOnce("disk is full");
    useUi.getState().openSettings("appearance");
    renderShell();
    await user.click(screen.getByRole("radio", { name: "High contrast" }));
    expect((await screen.findAllByRole("alert"))[0]).toHaveTextContent(
      "Settings could not be saved: disk is full",
    );
  });
});

describe("applyAppearance", () => {
  it("puts the theme, accent, density, scale and motion preference on the root element", () => {
    const root = document.createElement("html");
    applyAppearance(
      {
        ...defaults,
        appearance: {
          theme: "high-contrast",
          accent: "#1a237e",
          uiScale: 1.25,
          reducedMotion: true,
          compact: true,
        },
      },
      root,
    );
    expect(root.dataset.theme).toBe("high-contrast");
    expect(root.style.getPropertyValue("--accent")).toBe("#1a237e");
    expect(root.style.getPropertyValue("--accent-fg")).toBe("#ffffff");
    expect(root.style.fontSize).toBe("20px");
    expect(root.dataset.reducedMotion).toBe("");
    expect(root.dataset.density).toBe("compact");

    applyAppearance(defaults, root);
    expect(root.dataset.reducedMotion).toBeUndefined();
    expect(root.dataset.density).toBeUndefined();
  });
});
