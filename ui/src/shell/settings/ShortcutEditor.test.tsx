import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Tooltip } from "radix-ui";
import { describe, expect, it } from "vitest";

import { useUi } from "../../stores/ui";
import { connectTo, readyBackend } from "../../test/fakes";
import { SettingsDialog } from "./SettingsDialog";

async function openShortcuts() {
  const backend = readyBackend().reply("settings_set", ({ value }) => value);
  await connectTo(backend);
  useUi.getState().openSettings("shortcuts");
  render(
    <Tooltip.Provider>
      <SettingsDialog />
    </Tooltip.Provider>,
  );
  return backend;
}

const row = (title: string) => within(screen.getByRole("listitem", { name: title }));

describe("the shortcut editor", () => {
  it("records a new key for a command", async () => {
    const backend = await openShortcuts();
    await userEvent.click(
      row("Keyboard shortcuts").getByRole("button", { name: /Add a shortcut/ }),
    );
    expect(screen.getByText("Press keys, Esc to cancel")).toBeInTheDocument();
    await userEvent.keyboard("{Control>}{Alt>}s{/Alt}{/Control}");

    expect(backend.callsTo("settings_set").at(-1)).toMatchObject({
      section: "shortcuts",
      value: { bindings: { "app.shortcuts": ["Mod+Alt+S"] } },
    });
  });

  it("asks before taking a key another command uses", async () => {
    const backend = await openShortcuts();
    await userEvent.click(
      row("Keyboard shortcuts").getByRole("button", { name: /Add a shortcut/ }),
    );
    await userEvent.keyboard("{Control>}k{/Control}");

    const clash = screen.getByRole("alertdialog", { name: "Shortcut already in use" });
    expect(clash).toHaveTextContent("is already used by Command palette");
    expect(backend.callsTo("settings_set")).toHaveLength(0);

    await userEvent.click(within(clash).getByRole("button", { name: /Use it for/ }));
    expect(backend.callsTo("settings_set").at(-1)).toMatchObject({
      value: { bindings: { "app.shortcuts": ["Mod+K"], "app.palette": [] } },
    });
  });

  it("cancels recording with escape without closing settings", async () => {
    const backend = await openShortcuts();
    await userEvent.click(row("Settings").getByRole("button", { name: /Add a shortcut/ }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByText("Press keys, Esc to cancel")).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(backend.callsTo("settings_set")).toHaveLength(0);
  });

  it("switches presets", async () => {
    const backend = await openShortcuts();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Shortcut preset" }),
      "photoshop",
    );
    expect(backend.callsTo("settings_set").at(-1)).toMatchObject({
      section: "shortcuts",
      value: { preset: "photoshop" },
    });
    // the settings shortcut follows the preset
    expect(row("Settings").getByText("Ctrl+K")).toBeInTheDocument();
  });
});
