import type { Locator, Page } from "@playwright/test";

/** the same events the desktop app reports while files are dragged in */
type FileDropEvent =
  | { type: "enter"; paths: string[]; x: number; y: number }
  | { type: "over"; x: number; y: number }
  | { type: "drop"; paths: string[]; x: number; y: number }
  | { type: "leave" };

interface Harness {
  drag(event: FileDropEvent): void;
  quit(): void;
  nextPick: string[];
}

/** opens the app with nothing remembered from earlier tests */
export async function openFresh(page: Page, recent: string[] = []): Promise<void> {
  const query = new URLSearchParams([["fresh", ""], ...recent.map((path) => ["recent", path])]);
  await page.goto(`/e2e/harness/index.html?${query.toString()}`);
  await page.getByRole("heading", { name: "Vellum" }).waitFor();
}

/** opens the app as it is, which is how a restart after a crash looks */
export async function reopen(page: Page): Promise<void> {
  await page.goto("/e2e/harness/index.html");
}

// these run inside the page, so they can only use what they are passed

export async function sendDrag(page: Page, event: FileDropEvent): Promise<void> {
  await page.evaluate((payload) => {
    const harness = (window as unknown as { vellumHarness?: Harness }).vellumHarness;
    if (!harness) throw new Error("the harness is not loaded");
    harness.drag(payload);
  }, event);
}

async function centre(target: Locator): Promise<{ x: number; y: number }> {
  const box = await target.boundingBox();
  if (!box) throw new Error("the drop target is not on screen");
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** drags files in from the desktop and holds them over `target` */
export async function hover(page: Page, paths: string[], target: Locator): Promise<void> {
  await sendDrag(page, { type: "enter", paths, ...(await centre(target)) });
}

/** drags files in from the desktop and drops them on `target` */
export async function dropOn(page: Page, paths: string[], target: Locator): Promise<void> {
  const point = await centre(target);
  await sendDrag(page, { type: "enter", paths, ...point });
  await sendDrag(page, { type: "drop", paths, ...point });
}

/** the app exits cleanly, so the next start offers nothing to restore */
export async function quit(page: Page): Promise<void> {
  await page.evaluate(() => {
    const harness = (window as unknown as { vellumHarness?: Harness }).vellumHarness;
    if (!harness) throw new Error("the harness is not loaded");
    harness.quit();
  });
}

export async function setNextPick(page: Page, paths: string[]): Promise<void> {
  await page.evaluate((next) => {
    const harness = (window as unknown as { vellumHarness?: Harness }).vellumHarness;
    if (!harness) throw new Error("the harness is not loaded");
    harness.nextPick = next;
  }, paths);
}

/** the document tabs, not the tabs of docked panels */
export function fileTabs(page: Page): Locator {
  return page.getByRole("tablist", { name: "Open files" }).getByRole("tab");
}
