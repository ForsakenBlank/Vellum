import { expect, test } from "@playwright/test";

import { dropOn, openFresh, quit, reopen, fileTabs } from "./helpers";

test.describe("crash recovery", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page);
    await dropOn(
      page,
      ["/m/poster.png", "/m/trailer.mp4"],
      page.getByText("Recent", { exact: true }),
    );
    await expect(fileTabs(page)).toHaveCount(2);
  });

  test("after a crash the open files can be restored", async ({ page }) => {
    // reloading without a clean exit is what a crash looks like to the next start
    await reopen(page);
    const prompt = page.getByRole("alertdialog", { name: "Restore your last session?" });
    await expect(prompt).toBeVisible();
    await expect(prompt.getByRole("list", { name: "Files to restore" })).toHaveText(
      /poster\.png.*trailer\.mp4/s,
    );

    await prompt.getByRole("button", { name: "Restore" }).click();
    await expect(prompt).toBeHidden();
    await expect(fileTabs(page)).toHaveText([/poster\.png/, /trailer\.mp4/]);
  });

  test("starting fresh forgets the crashed session", async ({ page }) => {
    await reopen(page);
    await page.getByRole("button", { name: "Start fresh" }).click();
    await expect(page.getByRole("alertdialog")).toBeHidden();
    await expect(fileTabs(page)).toHaveCount(0);
  });

  test("a clean exit offers nothing on the next start", async ({ page }) => {
    await quit(page);
    await reopen(page);
    await expect(page.getByRole("heading", { name: "Vellum" })).toBeVisible();
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
  });
});
