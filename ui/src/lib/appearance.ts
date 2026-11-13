import type { AppearanceSettings } from "../bindings/AppearanceSettings";

const BASE_FONT_PX = 16;

/** pushes appearance settings onto the document root, where the css tokens read them */
export function applyAppearance(
  appearance: AppearanceSettings,
  root: HTMLElement = document.documentElement,
): void {
  root.dataset.theme = appearance.theme;
  root.dataset.compact = String(appearance.compact);
  root.dataset.reducedMotion = String(appearance.reducedMotion);
  root.style.setProperty("--v-accent", appearance.accent);
  // every size in the ui is in rem, so this one number scales all of it
  root.style.fontSize = `${BASE_FONT_PX * appearance.uiScale}px`;
}
