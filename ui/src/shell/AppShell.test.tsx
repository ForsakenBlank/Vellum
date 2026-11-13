import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { App } from "../App";
import { connect } from "../lib/connect";
import { useApp } from "../stores/app";
import { readyBackend } from "../test/fakes";
import { workspaces } from "../workspaces";

describe("the app shell", () => {
  it("has a rail entry for every workspace and opens its placeholder", async () => {
    render(<App />);
    const rail = screen.getByRole("navigation", { name: "Workspaces" });
    expect(rail.querySelectorAll("button")).toHaveLength(workspaces.length + 1);

    await userEvent.click(screen.getByRole("button", { name: "Video" }));
    expect(screen.getByRole("heading", { name: "Video" })).toBeInTheDocument();
    expect(screen.getByText("This workspace is not built yet.")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(screen.getByRole("heading", { name: "Vellum" })).toBeInTheDocument();
  });

  it("changes the theme through the settings backend", async () => {
    const backend = readyBackend().reply("settings_set", ({ value }) => value);
    await connect(backend);
    render(<App />);

    await userEvent.click(screen.getByRole("radio", { name: "Light" }));

    expect(backend.callsTo("settings_set")[0]).toMatchObject({
      section: "appearance",
      value: { theme: "light" },
    });
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("starts sample jobs from the home page", async () => {
    const backend = readyBackend();
    await connect(backend);
    render(<App />);

    await userEvent.click(screen.getByRole("button", { name: "Run a sample job" }));
    expect(backend.callsTo("dev_sample_job")).toEqual([{ seconds: 8, fail: false }]);
  });

  it("points to the desktop app when opened in a browser", async () => {
    await connect(null);
    render(<App />);
    expect(screen.getByText(/pnpm tauri dev/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run a sample job" })).toBeDisabled();
  });

  it("shows a notice that can be dismissed", async () => {
    useApp.setState({ notice: "Your settings could not be saved." });
    render(<App />);
    expect(screen.getByRole("alert")).toHaveTextContent("Your settings could not be saved.");

    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
