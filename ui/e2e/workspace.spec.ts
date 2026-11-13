import { expect, test } from "@playwright/test";

import { dropOn, fileTabs, openFresh, setNextPick } from "./helpers";

test.describe("tabs, panels and shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page);
    await dropOn(
      page,
      ["/m/a.mp4", "/m/b.mp4", "/m/c.mp4"],
      page.getByText("Recent", { exact: true }),
    );
  });

  test("tabs reorder by dragging along the bar", async ({ page }) => {
    const tabs = page.getByRole("tablist", { name: "Open files" }).getByRole("tab");
    await expect(tabs).toHaveText([/a\.mp4/, /b\.mp4/, /c\.mp4/]);
    const first = await tabs.nth(0).boundingBox();
    const last = await tabs.nth(2).boundingBox();
    if (!first || !last) throw new Error("tabs are not on screen");

    await page.mouse.move(first.x + 20, first.y + first.height / 2);
    await page.mouse.down();
    await page.mouse.move(last.x + last.width - 4, last.y + last.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect(tabs).toHaveText([/b\.mp4/, /c\.mp4/, /a\.mp4/]);
  });

  test("dragging a tab away from the bar tears it off into a new window", async ({ page }) => {
    const tab = fileTabs(page).filter({ hasText: /b\.mp4/ });
    const box = await tab.boundingBox();
    if (!box) throw new Error("the tab is not on screen");
    await page.mouse.move(box.x + 20, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + 60, box.y + 300, { steps: 8 });
    await expect(page.getByText("Release to open in a new window")).toBeVisible();
    await page.mouse.up();
    // it now lives in its own window, so this one no longer shows it
    await expect(fileTabs(page).filter({ hasText: /b\.mp4/ })).toBeHidden();
    await expect(fileTabs(page)).toHaveCount(2);
  });

  test("ctrl+w closes the tab in front and ctrl+shift+t brings it back", async ({ page }) => {
    await page.keyboard.press("Control+w");
    await expect(fileTabs(page)).toHaveText([/a\.mp4/, /b\.mp4/]);
    await page.keyboard.press("Control+Shift+T");
    await expect(fileTabs(page)).toHaveText([/a\.mp4/, /b\.mp4/, /c\.mp4/]);
  });

  test("a panel dragged onto another stack docks next to it", async ({ page }) => {
    const inspector = page.getByRole("tab", { name: "Inspector" });
    const box = await inspector.boundingBox();
    if (!box) throw new Error("the panel tab is not on screen");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    // over the timeline's left half, so it lands before it
    const timeline = await page.getByRole("region", { name: "Timeline" }).boundingBox();
    if (!timeline) throw new Error("the timeline is not on screen");
    await page.mouse.move(timeline.x + 30, timeline.y + timeline.height / 2, { steps: 10 });
    await page.mouse.up();

    const bottom = page.getByRole("complementary", { name: "Bottom panels" });
    await expect(bottom.getByRole("tab", { name: "Inspector" })).toBeVisible();
    await expect(bottom.getByRole("tab", { name: "Timeline" })).toBeVisible();
  });

  test("the open button asks for files and opens them", async ({ page }) => {
    await setNextPick(page, ["/m/song.flac"]);
    await page.getByRole("button", { name: "Open file" }).click();
    await expect(fileTabs(page).filter({ hasText: /song\.flac/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("the layout menu hides and shows panels", async ({ page }) => {
    await page.getByRole("button", { name: "Panels and layouts" }).click();
    await page.getByRole("menuitemcheckbox", { name: "Timeline" }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("complementary", { name: "Bottom panels" })).toBeHidden();

    await page.keyboard.press("Control+\\");
    await expect(page.getByRole("complementary", { name: "Left panels" })).toBeHidden();
    await page.keyboard.press("Control+\\");
    await expect(page.getByRole("complementary", { name: "Left panels" })).toBeVisible();
  });
});
