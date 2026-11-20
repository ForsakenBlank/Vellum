import type { Page } from "@playwright/test";

import { centreOf, dragFiles, dropFiles, expect, mod, test } from "./support";

async function openTool(page: Page, tool: string, files: string[]) {
  const card = await centreOf(page, `[data-drop-tool="${tool}"]`);
  await dragFiles(page, files, card.x, card.y);
  await dropFiles(page, files, card.x, card.y);
}

async function finishJobs(page: Page) {
  await page.evaluate(() => {
    (window as unknown as { __fake: { finishJobs: () => void } }).__fake.finishJobs();
  });
}

test.describe("export to size", () => {
  test("plans, runs and hands back a file to paste", async ({ page }) => {
    await openTool(page, "export_to_size", ["/media/holiday.mp4"]);
    const run = page.getByRole("button", { name: "Export to 10 MB" });
    await expect(run).toBeVisible();
    await expect(page.getByText("1280×720, 1180 kbps picture")).toBeVisible();

    // a preset chip changes the target, and undo puts it back
    await page.getByRole("button", { name: /Discord\s*20 MiB/ }).click();
    await expect(page.getByRole("button", { name: "Export to 20 MiB" })).toBeVisible();
    await page.getByRole("button", { name: /^Undo/ }).click();
    await expect(run).toBeVisible();

    await page.getByRole("radio", { name: "Keep frame rate" }).click();
    await expect(page.getByText("(keepFps)")).toBeVisible();

    await run.click();
    const files = page.getByRole("complementary", { name: "Files" });
    await expect(files.getByRole("progressbar")).toBeVisible();
    await expect(files.getByText("Encoding (pass 2 of 2)")).toBeVisible();

    await finishJobs(page);
    await expect(files.getByText("9.4 MB · 80% smaller")).toBeVisible();
    await expect(files.getByText("9.4 MB of the 9.7 MB allowed")).toBeVisible();
    await expect(files.getByText(/Scaled down from 1920×1080/)).toBeVisible();

    await files.getByRole("button", { name: "Copy as file" }).click();
    await expect(page.getByText("Paste holiday (10 MB).mp4 into a chat")).toBeVisible();
    const copied = await page.evaluate(
      () => (window as unknown as { __fake: { copied: string[] } }).__fake.copied,
    );
    expect(copied).toEqual(["/media/holiday (10 MB).mp4"]);
  });

  test("files dropped on the list join the batch", async ({ page }) => {
    await openTool(page, "export_to_size", ["/media/a.mp4"]);
    const files = page.getByRole("complementary", { name: "Files" });
    await expect(files.getByText("1 file", { exact: true })).toBeVisible();

    const list = await centreOf(page, '[aria-label="Files"]');
    await dragFiles(page, ["/media/b.png", "/media/c.wav"], list.x, list.y);
    await expect(page.getByText("Add 2 files to this batch")).toBeVisible();
    await dropFiles(page, ["/media/b.png", "/media/c.wav"], list.x, list.y);
    await expect(files.getByText("3 files", { exact: true })).toBeVisible();

    await files.getByRole("button", { name: "Remove from the batch" }).first().click();
    await expect(files.getByText("2 files", { exact: true })).toBeVisible();
    await expect(files.getByText("a.mp4")).toHaveCount(0);
  });
});

test.describe("video to gif", () => {
  test("estimates the size and switches format", async ({ page }) => {
    await openTool(page, "video_to_gif", ["/media/holiday.mp4"]);
    await expect(page.getByText("About")).toBeVisible();
    await expect(page.getByText("2.4 MB")).toBeVisible();
    await expect(page.getByRole("slider", { name: "Start of the clip" })).toBeVisible();

    await page.getByRole("radio", { name: "320" }).click();
    await expect(page.getByText(/320×270/)).toBeVisible();

    await page.getByLabel("Top text").fill("when it compiles");
    await page.getByRole("radio", { name: "WebP" }).click();
    const run = page.getByRole("button", { name: "Make WebP" });
    await expect(run).toBeVisible();

    await run.click();
    await finishJobs(page);
    await expect(page.getByText(/As an MP4 this would be about 0.6 MB/)).toBeVisible();
  });

  test("the trim handles move by one frame with the arrow keys", async ({ page }) => {
    await openTool(page, "video_to_gif", ["/media/holiday.mp4"]);
    const start = page.getByRole("slider", { name: "Start of the clip" });
    await start.focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    // 29.97 frames a second, two frames in
    await expect(start).toHaveAttribute("aria-valuetext", "0:00.07");
    await page.keyboard.press("Shift+ArrowRight");
    await expect(start).toHaveAttribute("aria-valuetext", "0:01.07");
  });
});

test.describe("convert", () => {
  test("says when it can copy instead of re-encoding", async ({ page }) => {
    await openTool(page, "convert", ["/media/holiday.mp4"]);
    await page.getByRole("radio", { name: /^MKV/ }).click();
    await expect(page.getByText(/Instant: the streams already suit MKV/)).toBeVisible();
    await page.getByRole("button", { name: "Turn", exact: true }).click();
    await expect(page.getByText(/Instant/)).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Convert to MKV" })).toBeVisible();
  });
});

test.describe("export settings", () => {
  test("presets can be edited and reset", async ({ page }) => {
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("combobox").fill("size presets");
    await page
      .getByRole("option", { name: /Size presets/ })
      .first()
      .click();
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    const presets = page.getByRole("list", { name: "Size presets" });
    await expect(presets.getByLabel("Preset name")).toHaveCount(7);
    await presets.getByRole("button", { name: "Remove WhatsApp" }).click();
    await expect(presets.getByLabel("Preset name")).toHaveCount(6);
    await page.getByRole("button", { name: "Reset to defaults" }).click();
    await expect(presets.getByLabel("Preset name")).toHaveCount(7);
  });
});
