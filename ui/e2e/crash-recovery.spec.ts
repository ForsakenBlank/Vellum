import { expect, mod, nextDialog, test } from "./support";

async function openAndWrite(page: import("@playwright/test").Page, notes: string) {
  await nextDialog(page, ["/media/sunset.png"]);
  await page.keyboard.press(`${mod}+o`);
  await expect(page.getByRole("tab", { name: /sunset\.png/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const box = page.getByRole("textbox", { name: "Notes" });
  await box.fill(notes);
  await box.blur();
  await expect(page.getByLabel("Unsaved changes")).toBeVisible();
}

test.describe("crash recovery", () => {
  test("offers the autosaved session after a crash and restores it", async ({ page }) => {
    await openAndWrite(page, "warm the shadows");

    // reloading without a clean exit is what a crash looks like to the next launch
    await page.reload();
    const prompt = page.getByRole("dialog", { name: "Restore your last session?" });
    await expect(prompt).toBeVisible();
    await expect(prompt).toContainText("Vellum did not close properly");
    await expect(prompt.getByRole("list", { name: "Documents to restore" })).toContainText(
      "sunset.png",
    );
    await expect(prompt).toContainText("Unsaved changes");
    await expect(prompt.getByRole("button", { name: /Restore document/ })).toBeFocused();

    await prompt.getByRole("button", { name: /Restore document/ }).click();
    await expect(prompt).toBeHidden();
    await expect(page.getByRole("tab", { name: /sunset\.png/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByRole("textbox", { name: "Notes" })).toHaveValue("warm the shadows");
    await expect(page.getByLabel("Unsaved changes")).toBeVisible();
    await expect(page.getByText("Restored 1 document")).toBeVisible();
  });

  test("discarding forgets the session for good", async ({ page }) => {
    await openAndWrite(page, "draft");
    await page.reload();
    await page.getByRole("button", { name: "Discard" }).click();
    await expect(page.getByRole("dialog")).toBeHidden();
    await expect(page.getByRole("tab", { name: /sunset\.png/ })).toHaveCount(0);

    await page.evaluate(() => {
      (window as unknown as { __fake: { cleanExit: () => void } }).__fake.cleanExit();
    });
    await page.reload();
    await expect(page.getByRole("heading", { name: "Drop anything to start" })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("quitting with unsaved work offers it back, with different words", async ({ page }) => {
    await openAndWrite(page, "keep me");
    await page.evaluate(() => {
      (window as unknown as { __fake: { cleanExit: () => void } }).__fake.cleanExit();
    });
    await page.reload();
    const prompt = page.getByRole("dialog", { name: "Pick up where you left off?" });
    await expect(prompt).toContainText("closed with unsaved changes");
  });

  test("a clean quit with everything saved starts fresh", async ({ page }) => {
    await openAndWrite(page, "saved notes");
    await page.keyboard.press(`${mod}+s`);
    await expect(page.getByLabel("Unsaved changes")).toHaveCount(0);
    await page.evaluate(() => {
      (window as unknown as { __fake: { cleanExit: () => void } }).__fake.cleanExit();
    });
    await page.reload();
    await expect(page.getByRole("heading", { name: "Drop anything to start" })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("undo and redo work from the shortcuts and the edit menu", async ({ page }) => {
    await openAndWrite(page, "first");
    const box = page.getByRole("textbox", { name: "Notes" });
    await box.fill("first, second");
    await box.blur();

    await page.keyboard.press(`${mod}+z`);
    await expect(box).toHaveValue("first");
    await page.getByRole("menuitem", { name: "Edit" }).click();
    await page.getByRole("menuitem", { name: /^Redo Edit notes/ }).click();
    await expect(box).toHaveValue("first, second");
  });
});
