import { dropFiles, expect, test } from "./support";

test.describe("the info panel", () => {
  test("shows what is inside a photo and offers a clean copy", async ({ page }) => {
    await dropFiles(page, ["/media/photo.jpg"], 600, 300);
    const panel = page.getByRole("region", { name: "Info" });

    await expect(panel.getByText("4032 × 3024")).toBeVisible();
    await expect(panel.getByText("1 tag, 1 may be private")).toBeVisible();

    await panel.getByRole("button", { name: "Show tags" }).click();
    await expect(panel.getByText("GPSLatitude")).toBeVisible();

    await panel.getByRole("button", { name: "Strip metadata" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Removing metadata" })).toBeVisible();
  });

  test("lists the file's details for a video", async ({ page }) => {
    await dropFiles(page, ["/media/holiday.mp4"], 600, 300);
    const panel = page.getByRole("region", { name: "Info" });
    await expect(panel.getByText("MP4 file")).toBeVisible();
    await expect(panel.getByText("0:12")).toBeVisible();
    await expect(panel.getByText("No hidden metadata.")).toBeVisible();
  });
});
