import { expect, test } from "@playwright/test";

import { dropOn, openFresh, fileTabs } from "./helpers";

test.describe("the command palette", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page, ["/home/me/Videos/holiday.mp4", "/home/me/Pictures/logo.png"]);
  });

  test("opens with ctrl+k, finds commands fuzzily and runs them", async ({ page }) => {
    await page.keyboard.press("Control+k");
    const search = page.getByRole("combobox", { name: "Search commands, settings and files" });
    await expect(search).toBeFocused();

    await search.fill("go vid");
    await expect(page.getByRole("option").first()).toContainText("Go to Video");
    await page.keyboard.press("Enter");

    await expect(search).toBeHidden();
    await expect(page.getByText("Nothing open in Video")).toBeVisible();
  });

  test("moves through results with the arrow keys", async ({ page }) => {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("theme");
    const options = page.getByRole("option");
    await expect(options.first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowUp");
    await expect(options.first()).toHaveAttribute("aria-selected", "true");
  });

  test("finds recent files and opens them", async ({ page }) => {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("holi");
    await expect(page.getByRole("option").first()).toContainText("holiday.mp4");
    await page.keyboard.press("Enter");
    await expect(fileTabs(page).filter({ hasText: /holiday\.mp4/ })).toBeVisible();
  });

  test("finds settings and opens settings at them", async ({ page }) => {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("interface size");
    await page.keyboard.press("Enter");
    const settings = page.getByRole("dialog", { name: "Settings" });
    await expect(settings.getByRole("slider", { name: "Interface size" })).toBeVisible();
  });

  test("applies a shortcut preset", async ({ page }) => {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("photoshop");
    await page.getByRole("option", { name: /Use Photoshop shortcuts/ }).click();
    // photoshop opens preferences with ctrl+k, so the palette moves to ctrl+shift+p
    await page.keyboard.press("Control+k");
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await page.keyboard.press("Control+Shift+P");
    await expect(page.getByRole("combobox", { name: /Search commands/ })).toBeVisible();
  });

  test("closes with escape and with the same shortcut", async ({ page }) => {
    await page.keyboard.press("Control+k");
    await expect(page.getByRole("combobox")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("combobox")).toBeHidden();
    await page.keyboard.press("Control+k");
    await page.keyboard.press("Control+k");
    await expect(page.getByRole("combobox")).toBeHidden();
  });

  test("closes the open tab from the palette", async ({ page }) => {
    await dropOn(page, ["/m/a.png"], page.getByText("Recent", { exact: true }));
    await expect(fileTabs(page).filter({ hasText: /a\.png/ })).toBeVisible();
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("close tab");
    await page.keyboard.press("Enter");
    await expect(fileTabs(page).filter({ hasText: /a\.png/ })).toBeHidden();
  });
});
