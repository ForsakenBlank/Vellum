import { dropFiles, expect, test } from "./support";

test.describe("info panel", () => {
  test("shows what the file is and offers to strip its metadata", async ({ page }) => {
    await dropFiles(page, ["/media/beach.mp4"], 600, 300);
    const info = page.getByRole("region", { name: "Info" });

    await expect(info.getByText("This file records where it was made")).toBeVisible();
    await expect(info).toContainText("1080 × 1920");
    await expect(info).toContainText("21.4 s");
    await expect(info).toContainText("29.97 fps, variable");
    await expect(info).toContainText("HLG");
    await expect(info).toContainText("90° clockwise");
    await expect(info).toContainText("Stereo");

    await info.getByText("Metadata (3)").click();
    await expect(info).toContainText("com.apple.quicktime.make");

    await info.getByRole("button", { name: "Strip metadata" }).click();
    await expect(page.getByText("Stripping metadata")).toBeVisible();
  });

  test("says plainly when a file cannot be read", async ({ page }) => {
    await dropFiles(page, ["/media/broken.mov"], 600, 300);
    const info = page.getByRole("region", { name: "Info" });
    await expect(info).toContainText("the e2e backend can only read mp4 files");
    await expect(info.getByRole("button", { name: "Strip metadata" })).toHaveCount(0);
  });
});
