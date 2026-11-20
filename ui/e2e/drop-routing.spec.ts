import { centreOf, dragFiles, dropFiles, expect, test } from "./support";

test.describe("dropping files", () => {
  test("a video dropped anywhere opens in the video workspace", async ({ page }) => {
    await dragFiles(page, ["/media/holiday.mp4"], 600, 300);
    const overlay = page.getByTestId("drop-overlay");
    await expect(overlay).toContainText("Open holiday.mp4 in Video");
    await expect(overlay).toContainText("A video file.");

    await dropFiles(page, ["/media/holiday.mp4"], 600, 300);
    await expect(overlay).toBeHidden();
    await expect(page.getByRole("tab", { name: /holiday\.mp4/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(
      page.getByRole("navigation", { name: "Workspaces" }).getByRole("button", { name: "Video" }),
    ).toHaveAttribute("aria-current", "true");
    await expect(page.getByText("The video editor is on its way")).toBeVisible();
  });

  test("several files each open in their own workspace", async ({ page }) => {
    const files = ["/media/cover.png", "/media/theme.mp3", "/media/readme.txt"];
    await dragFiles(page, files, 600, 300);
    await expect(page.getByTestId("drop-overlay")).toContainText("Open 2 files in Image, Audio");
    await expect(page.getByTestId("drop-overlay")).toContainText("readme.txt will be skipped.");
    await dropFiles(page, files, 600, 300);

    await expect(page.getByRole("tab", { name: /cover\.png/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByRole("tab", { name: /theme\.mp3/ })).toBeVisible();
    await expect(page.getByText("The image editor is on its way")).toBeVisible();
  });

  test("a video dropped on the GIF card opens that tool", async ({ page }) => {
    const card = await centreOf(page, '[data-drop-tool="video_to_gif"]');
    await dragFiles(page, ["/media/clip.mov"], card.x, card.y);
    await expect(page.getByTestId("drop-overlay")).toContainText("Make a GIF from clip.mov");
    await expect(page.locator('[data-drop-tool="video_to_gif"]')).toContainText(
      "Make a GIF from clip.mov",
    );

    await dropFiles(page, ["/media/clip.mov"], card.x, card.y);
    await expect(page.getByRole("tab", { name: /Video to GIF: clip\.mov/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByRole("button", { name: "Make GIF" })).toBeVisible();
  });

  test("a card says no to files it cannot use, and nothing opens", async ({ page }) => {
    const card = await centreOf(page, '[data-drop-tool="video_to_gif"]');
    await dragFiles(page, ["/media/photo.jpg"], card.x, card.y);
    await expect(page.getByTestId("drop-overlay")).toContainText("Video to GIF needs a video");

    await dropFiles(page, ["/media/photo.jpg"], card.x, card.y);
    await expect(
      page.getByRole("status").filter({ hasText: "Video to GIF needs a video" }),
    ).toBeVisible();
    await expect(
      page.getByRole("tablist", { name: "Open documents" }).getByRole("tab"),
    ).toHaveCount(1);
  });

  test("files vellum cannot open are explained", async ({ page }) => {
    await dragFiles(page, ["/media/notes.txt"], 600, 300);
    await expect(page.getByTestId("drop-overlay")).toContainText(
      "Vellum cannot open .txt files yet",
    );
    await page.evaluate(() => {
      (window as unknown as { __fake: { drag: (...a: unknown[]) => void } }).__fake.drag(
        "leave",
        [],
        0,
        0,
      );
    });
    await expect(page.getByTestId("drop-overlay")).toBeHidden();
  });

  test("dropping a file that is already open switches to its tab", async ({ page }) => {
    await dropFiles(page, ["/media/a.png"], 600, 300);
    await page.getByRole("tab", { name: "Home" }).click();
    await dropFiles(page, ["/media/a.png"], 600, 300);
    await expect(page.getByRole("tab", { name: /a\.png/ })).toHaveCount(1);
    await expect(page.getByRole("tab", { name: /a\.png/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});
