import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Tooltip } from "radix-ui";
import { describe, expect, it } from "vitest";

import type { DockLayout } from "../bindings/DockLayout";
import { useSettings } from "../stores/settings";
import { connectTo, readyBackend } from "../test/fakes";
import { videoWorkspace } from "../workspaces/video";
import { Dock } from "./Dock";

async function showVideo() {
  const backend = readyBackend().reply("settings_set", ({ value }) => value);
  await connectTo(backend);
  render(
    <Tooltip.Provider>
      <Dock workspace={videoWorkspace}>
        <p>middle</p>
      </Dock>
    </Tooltip.Provider>,
  );
  return backend;
}

function savedLayout(): DockLayout | undefined {
  return useSettings.getState().sections?.layouts.current.video;
}

describe("the dock", () => {
  it("lays out the workspace's default panels around the middle", async () => {
    await showVideo();
    expect(screen.getByText("middle")).toBeInTheDocument();
    const left = screen.getByRole("complementary", { name: "Left panels" });
    expect(within(left).getByRole("tab", { name: "Media" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(left).getByRole("tab", { name: "Effects" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(screen.getByRole("complementary", { name: "Bottom panels" })).toBeInTheDocument();
  });

  it("switches tabs and saves the layout", async () => {
    await showVideo();
    await userEvent.click(screen.getByRole("tab", { name: "Effects" }));
    expect(savedLayout()?.left.stacks[0]?.active).toBe("effects");
    expect(screen.getByRole("tab", { name: "Effects" })).toHaveAttribute("aria-selected", "true");
  });

  it("moves and closes panels from the panel menu", async () => {
    await showVideo();
    const bottom = screen.getByRole("complementary", { name: "Bottom panels" });
    await userEvent.click(within(bottom).getByRole("button", { name: "Panel options" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Dock on the right" }));
    expect(savedLayout()?.right.stacks.map((stack) => stack.panels)).toEqual([
      ["inspector", "history"],
      ["timeline"],
    ]);
    expect(screen.queryByRole("complementary", { name: "Bottom panels" })).not.toBeInTheDocument();

    const right = screen.getByRole("complementary", { name: "Right panels" });
    const [, second] = within(right).getAllByRole("button", { name: "Panel options" });
    if (!second) throw new Error("the timeline stack should be second");
    await userEvent.click(second);
    await userEvent.click(screen.getByRole("menuitem", { name: "Close Timeline" }));
    expect(savedLayout()?.right.stacks.map((stack) => stack.panels)).toEqual([
      ["inspector", "history"],
    ]);
  });
});
