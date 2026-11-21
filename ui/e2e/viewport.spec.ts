import { dropFiles, expect, mod, test } from "./support";

const canvas = '[data-testid="gpu-viewport"]';

test.describe("gpu viewport", () => {
  test.beforeEach(async ({ page }) => {
    await dropFiles(page, ["/photos/mountain.png"], 600, 300);
    await expect(page.locator(canvas)).toBeVisible();
    await expect
      .poll(async () => Number(await page.locator(canvas).getAttribute("data-tiles")))
      .toBeGreaterThan(0);
  });

  test("shows the picture's size and profile", async ({ page }) => {
    await expect(page.getByText("6000 × 4000 px · Display P3")).toBeVisible();
    await expect(page.locator(canvas)).toHaveAttribute("aria-label", /mountain\.png at \d/);
  });

  test("keeps up with the display while panning, zooming and turning", async ({ page }) => {
    // drive the canvas from inside the page, one change per animation frame, so the
    // measurement is the viewport's own speed and not playwright's round trips
    const result = await page.evaluate(async (selector) => {
      const el = document.querySelector<HTMLCanvasElement>(selector);
      if (!el) throw new Error("no viewport");
      const rect = el.getBoundingClientRect();
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      const startFrames = Number(el.dataset.frames);
      const point = (i: number) => ({
        clientX: cx + Math.sin(i / 9) * 150,
        clientY: cy + Math.cos(i / 11) * 100,
        bubbles: true,
        cancelable: true,
      });
      el.dispatchEvent(new PointerEvent("pointerdown", { ...point(0), button: 0, pointerId: 7 }));
      const t0 = performance.now();
      let i = 0;
      await new Promise<void>((resolve) => {
        const step = () => {
          if (performance.now() - t0 > 2000) {
            resolve();
            return;
          }
          i++;
          if (i % 3 === 0) {
            el.dispatchEvent(
              new WheelEvent("wheel", { ...point(i), deltaY: Math.sin(i / 7) * 60 }),
            );
          } else if (i % 3 === 1) {
            el.dispatchEvent(new PointerEvent("pointermove", { ...point(i), pointerId: 7 }));
          } else {
            el.dispatchEvent(new WheelEvent("wheel", { ...point(i), deltaY: 8, shiftKey: true }));
          }
          requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      });
      el.dispatchEvent(new PointerEvent("pointerup", { ...point(i), pointerId: 7 }));
      const seconds = (performance.now() - t0) / 1000;
      return {
        fps: (Number(el.dataset.frames) - startFrames) / seconds,
        drawMs: Number(el.dataset.drawMs),
      };
    }, canvas);
    console.log(`viewport: ${result.fps.toFixed(1)} fps, ${result.drawMs.toFixed(2)} ms per draw`);
    // about 57 on an idle machine, where headless chromium's frame loop tops out. other
    // tests running alongside share the processor, so the bar is lower than that; a
    // viewport that waited on rust between frames would fall far below it
    expect(result.fps).toBeGreaterThanOrEqual(30);
    expect(result.drawMs).toBeLessThan(4);
    // the view turned, so the straighten button shows the angle
    await expect(
      page.getByRole("toolbar", { name: "View controls" }).getByRole("button", { name: /°$/ }),
    ).toBeVisible();
  });

  test("zooms, straightens and mirrors from the keyboard and toolbar", async ({ page }) => {
    const view = page.locator(canvas);
    const bar = page.getByRole("toolbar", { name: "View controls" });
    await view.focus();
    // from the overview to actual pixels the window asks for finer tiles
    const before = Number(await view.getAttribute("data-tiles"));
    await page.keyboard.press(`${mod}+1`);
    await expect(bar.getByRole("button", { name: "100%" })).toBeVisible();
    await expect
      .poll(async () => Number(await view.getAttribute("data-tiles")))
      .toBeGreaterThan(before);
    await page.keyboard.press(`${mod}+=`);
    await expect(bar.getByRole("button", { name: "200%" })).toBeVisible();
    await page.keyboard.press(`${mod}+0`);
    await expect(bar.getByRole("button", { name: /^\d+(\.\d+)?%$/ })).not.toHaveText("200%");

    await bar.getByRole("button", { name: /Turn the view right/ }).click();
    await expect(bar.getByRole("button", { name: "15°" })).toBeVisible();
    await view.focus();
    await page.keyboard.press("Escape");
    await expect(bar.getByRole("button", { name: "15°" })).toHaveCount(0);

    const mirror = bar.getByRole("button", { name: /Mirror the view/ });
    await mirror.click();
    await expect(mirror).toHaveAttribute("aria-pressed", "true");
  });

  test("the app's own zoom shortcut still works away from the picture", async ({ page }) => {
    // with the picture focused, ctrl+= zooms the picture and not the interface
    await page.locator(canvas).focus();
    const scale = () => page.evaluate(() => document.documentElement.style.fontSize);
    const start = await scale();
    await page.keyboard.press(`${mod}+=`);
    expect(await scale()).toBe(start);
  });
});
