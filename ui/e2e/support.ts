import { test as base, expect, type Page } from "@playwright/test";

import fileKinds from "../src/bindings/fileKinds.json" with { type: "json" };
import fixture from "./fixtures/backend.json" with { type: "json" };
import { installFakeBackend } from "./fakeBackend";

/** every test starts with the fake backend installed and the app loaded */
export const test = base.extend({
  page: async ({ page }, provide) => {
    await page.addInitScript(installFakeBackend, { fixture, fileKinds, windowLabel: "main" });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Drop anything to start" })).toBeVisible();
    await provide(page);
  },
});

export { expect };

/** drags files over the window the way tauri reports it, ending over (x, y) */
export async function dragFiles(page: Page, paths: string[], x: number, y: number) {
  await page.evaluate(
    ({ paths, x, y }) => {
      const fake = (window as unknown as { __fake: { drag: (...args: unknown[]) => void } }).__fake;
      fake.drag("enter", paths, 10, 10);
      fake.drag("over", paths, x, y);
    },
    { paths, x, y },
  );
}

export async function dropFiles(page: Page, paths: string[], x: number, y: number) {
  await page.evaluate(
    ({ paths, x, y }) => {
      (window as unknown as { __fake: { drag: (...args: unknown[]) => void } }).__fake.drag(
        "drop",
        paths,
        x,
        y,
      );
    },
    { paths, x, y },
  );
}

/** the centre of an element, for aiming a drop at it */
export async function centreOf(page: Page, selector: string) {
  const box = await page.locator(selector).boundingBox();
  if (!box) throw new Error(`${selector} is not on screen`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** makes the next file dialog return these paths */
export async function nextDialog(page: Page, paths: string[]) {
  await page.evaluate((paths) => {
    (window as unknown as { __fake: { nextDialog: string[][] } }).__fake.nextDialog.push(paths);
  }, paths);
}

export const mod = process.platform === "darwin" ? "Meta" : "Control";
