import { expect, test } from "@playwright/test";

import { dropOn, hover, openFresh, sendDrag, fileTabs } from "./helpers";

test.describe("dropping files from the desktop", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page);
  });

  test("a file dropped anywhere opens in the workspace for its type", async ({ page }) => {
    await hover(page, ["/home/me/cat.jpg"], page.getByText("Recent", { exact: true }));
    await expect(page.getByRole("status").filter({ hasText: "Open in Image" })).toBeVisible();

    await sendDrag(page, { type: "drop", paths: ["/home/me/cat.jpg"], x: 600, y: 700 });
    await expect(fileTabs(page).filter({ hasText: /cat\.jpg/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByRole("heading", { name: "cat.jpg" })).toBeVisible();
    await expect(page.getByText("Editing in Image arrives in phase 5.")).toBeVisible();
    await expect(
      page.getByRole("navigation", { name: "Workspaces" }).getByRole("button", { name: "Image" }),
    ).toHaveAttribute("aria-current", "page");
  });

  test("several files of different types each go to their own workspace", async ({ page }) => {
    const files = ["/m/a.png", "/m/b.mp4", "/m/c.wav"];
    await hover(page, files, page.getByText("Recent", { exact: true }));
    await expect(page.getByText("Open 3 files in 3 workspaces")).toBeVisible();
    await sendDrag(page, { type: "drop", paths: files, x: 600, y: 700 });

    const tabs = page.getByRole("tablist", { name: "Open files" }).getByRole("tab");
    await expect(tabs).toHaveText([/a\.png/, /b\.mp4/, /c\.wav/]);
    // the last one is in front
    await expect(page.getByText("Editing in Audio arrives in phase 11.")).toBeVisible();
  });

  test("a quick tool card takes the files it can use", async ({ page }) => {
    const card = page.getByRole("button", { name: /Video to GIF/ });
    await hover(page, ["/m/clip.mp4"], card);
    await expect(card).toContainText("Drop to make a gif");
    await expect(page.getByRole("status").filter({ hasText: "Make a GIF" })).toBeVisible();

    await dropOn(page, ["/m/clip.mp4"], card);
    await expect(fileTabs(page).filter({ hasText: /Video to GIF: clip\.mp4/ })).toBeVisible();
    await expect(page.getByText("Video to GIF arrives in phase 3.")).toBeVisible();
  });

  test("a quick tool says no to files it cannot use", async ({ page }) => {
    const card = page.getByRole("button", { name: /Remove background/ });
    await hover(page, ["/m/clip.mp4"], card);
    await expect(card).toContainText("Can't take this file");
    await expect(page.getByText("This file can't be opened")).toBeVisible();

    await dropOn(page, ["/m/clip.mp4"], card);
    await expect(page.getByRole("alert")).toContainText("clip.mp4 was not opened.");
    await expect(fileTabs(page)).toHaveCount(0);
  });

  test("a file dropped on a workspace icon opens there whatever its type", async ({ page }) => {
    const audio = page
      .getByRole("navigation", { name: "Workspaces" })
      .getByRole("button", { name: "Audio" });
    await dropOn(page, ["/m/interview.mp4"], audio);
    await expect(fileTabs(page).filter({ hasText: /interview\.mp4/ })).toBeVisible();
    await expect(page.getByText("Editing in Audio arrives in phase 11.")).toBeVisible();
  });

  test("files nobody can open are explained and skipped", async ({ page }) => {
    await hover(page, ["/m/notes.txt"], page.getByText("Recent", { exact: true }));
    await expect(page.getByText("Vellum does not know this file type yet.")).toBeVisible();
    await sendDrag(page, { type: "leave" });
    await expect(page.getByText("This file can't be opened")).toBeHidden();
  });
});
