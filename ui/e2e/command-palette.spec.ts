import fixture from "./fixtures/backend.json" with { type: "json" };
import { dropFiles, expect, mod, test } from "./support";

test.describe("command palette", () => {
  test("opens with the shortcut and runs a command", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    const search = page.getByRole("combobox", { name: /search commands/i });
    await expect(search).toBeFocused();
    await search.fill("light theme");
    await expect(page.getByRole("option").first()).toContainText("Light theme");
    await page.keyboard.press("Enter");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(search).toBeHidden();
  });

  test("lists every command rust registered", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await expect(page.getByRole("option").first()).toBeVisible();
    // nothing typed: every command is listed, after the recent ones
    const shown = await page.getByRole("option").allTextContents();
    for (const command of fixture.commands) {
      expect(shown.some((text) => text.startsWith(command.title))).toBe(true);
    }
  });

  test("moves with the arrow keys and closes with escape", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("zoom");
    await expect(page.getByRole("option").first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("option").nth(1)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("combobox")).toBeHidden();
  });

  test("finds recent files and opens them", async ({ page }) => {
    await dropFiles(page, ["/media/holiday.mp4"], 600, 300);
    await page.getByRole("tab", { name: /holiday\.mp4/ }).hover();
    await page.keyboard.press(`${mod}+w`);
    await expect(page.getByRole("tab", { name: /holiday\.mp4/ })).toHaveCount(0);

    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("holiday");
    await expect(page.getByRole("option").first()).toContainText("holiday.mp4");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("tab", { name: /holiday\.mp4/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("finds settings and opens them in place", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("compact");
    await page
      .getByRole("option", { name: /Compact mode/ })
      .filter({ hasText: "Appearance" })
      .click();
    const settings = page.getByRole("dialog", { name: "Settings" });
    await expect(settings).toBeVisible();
    await settings.getByRole("switch", { name: "Compact mode" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  });

  test("switches to a shortcut preset, and the new shortcuts work", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("photoshop shortcuts");
    await page.keyboard.press("Enter");
    // in the photoshop preset ctrl+k opens settings, like photoshop's preferences
    await page.keyboard.press(`${mod}+k`);
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await page.keyboard.press(`${mod}+Shift+p`);
    await expect(page.getByRole("combobox", { name: /search commands/i })).toBeVisible();
  });

  test("explains when nothing matches", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("qqqqqq");
    await expect(page.getByText(/Nothing matches/)).toBeVisible();
  });
});
