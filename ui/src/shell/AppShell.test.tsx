import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { App } from "../App";
import { useApp } from "../stores/app";
import { connectTo, doc, readyBackend, sessionWith } from "../test/fakes";
import { workspaces } from "../workspaces";

describe("the app shell", () => {
  it("has a rail entry for every workspace that asks the backend to switch", async () => {
    const backend = readyBackend();
    await connectTo(backend);
    render(<App />);
    const rail = screen.getByRole("navigation", { name: "Workspaces" });
    expect(within(rail).getAllByRole("button")).toHaveLength(workspaces.length + 1);

    await userEvent.click(within(rail).getByRole("button", { name: "Video" }));
    expect(backend.callsTo("shell_run")).toEqual([
      { record: { id: "shell.workspace", params: { window: "main", workspace: "video" } } },
    ]);
  });

  it("shows the workspace and document the session says this window shows", async () => {
    const backend = readyBackend();
    await connectTo(backend);
    render(<App />);
    expect(screen.getByRole("heading", { name: "Vellum" })).toBeInTheDocument();

    act(() => {
      backend.emit("session.changed", {
        session: sessionWith([doc({ id: 7 })], { type: "document", id: 7 }, 3),
      });
    });
    expect(screen.getByRole("tab", { name: /clip\.mp4/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "clip.mp4" })).toBeInTheDocument();
    expect(screen.getByText("Editing in Video arrives in phase 8.")).toBeInTheDocument();
    // the video workspace docks its own panels
    expect(screen.getByRole("tab", { name: "Timeline" })).toBeInTheDocument();

    act(() => {
      backend.emit("session.changed", {
        session: sessionWith([], { type: "workspace", id: "audio" }, 4),
      });
    });
    expect(screen.getByText("Nothing open in Audio")).toBeInTheDocument();
  });

  it("closes a tab through the backend", async () => {
    const backend = readyBackend();
    await connectTo(backend);
    render(<App />);
    act(() => {
      backend.emit("session.changed", {
        session: sessionWith([doc({ id: 7 })], { type: "document", id: 7 }, 3),
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Close clip.mp4" }));
    expect(backend.callsTo("shell_run").at(-1)).toEqual({
      record: { id: "shell.close", params: { id: 7 } },
    });
  });

  it("opens the palette with ctrl+k and runs what is picked", async () => {
    const backend = readyBackend().reply("settings_set", ({ value }) => value);
    await connectTo(backend);
    render(<App />);

    await userEvent.keyboard("{Control>}k{/Control}");
    const search = screen.getByRole("combobox", { name: "Search commands, settings and files" });
    await userEvent.type(search, "light theme");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("Use light theme");
    await userEvent.keyboard("{Enter}");

    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(backend.callsTo("settings_set")[0]).toMatchObject({
      section: "appearance",
      value: { theme: "light" },
    });
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("finds a setting in the palette and opens settings at it", async () => {
    await connectTo(readyBackend());
    render(<App />);
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(screen.getByRole("combobox"), "autosave");
    await userEvent.keyboard("{Enter}");

    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(within(dialog).getByRole("region", { name: "General" })).toBeInTheDocument();
    expect(within(dialog).getByRole("combobox", { name: "Autosave" })).toHaveValue("30");
  });

  it("offers to restore a crashed session and does what the user picks", async () => {
    const offer = { savedMs: Date.now(), documents: [doc({ title: "trip.mp4" })] };
    const backend = readyBackend()
      .reply("recovery_offer", () => offer)
      .reply("recovery_restore", () =>
        sessionWith([doc({ id: 9, title: "trip.mp4" })], { type: "document", id: 9 }, 2),
      );
    await connectTo(backend);
    render(<App />);

    const prompt = screen.getByRole("alertdialog", { name: "Restore your last session?" });
    expect(within(prompt).getByText("trip.mp4")).toBeInTheDocument();
    // the safe choice has the focus, so enter never throws work away
    expect(within(prompt).getByRole("button", { name: "Restore" })).toHaveFocus();
    await userEvent.click(within(prompt).getByRole("button", { name: "Restore" }));

    expect(backend.callsTo("recovery_restore")).toHaveLength(1);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /trip\.mp4/ })).toBeInTheDocument();
  });

  it("can start fresh instead of restoring", async () => {
    const offer = { savedMs: Date.now(), documents: [doc()] };
    const backend = readyBackend().reply("recovery_offer", () => offer);
    await connectTo(backend);
    render(<App />);
    await userEvent.click(screen.getByRole("button", { name: "Start fresh" }));
    expect(backend.callsTo("recovery_discard")).toHaveLength(1);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("points to the desktop app when opened in a browser", async () => {
    await connectTo(null);
    render(<App />);
    expect(screen.getByText(/pnpm tauri dev/)).toBeInTheDocument();
    for (const button of screen.getAllByRole("button", { name: "Open file" })) {
      expect(button).toBeDisabled();
    }
  });

  it("shows a notice that can be dismissed", async () => {
    useApp.setState({ notice: "Your settings could not be saved." });
    render(<App />);
    expect(screen.getByRole("alert")).toHaveTextContent("Your settings could not be saved.");

    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
