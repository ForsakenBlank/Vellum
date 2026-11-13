import type { DockLayout } from "../bindings/DockLayout";
import type { DockZone } from "../bindings/DockZone";
import type { PanelStack } from "../bindings/PanelStack";

/**
 * pure operations on a dock layout. every function returns a new layout and
 * leaves its input alone, so the dock can preview a change while dragging and
 * only save it on release.
 */

export type ZoneName = "left" | "right" | "bottom";
export const ZONES: ZoneName[] = ["left", "right", "bottom"];

/** sizes in rem, matching the clamps in the rust settings section */
export const ZONE_LIMITS: Record<ZoneName, { min: number; max: number }> = {
  left: { min: 10, max: 40 },
  right: { min: 10, max: 40 },
  bottom: { min: 6, max: 40 },
};

/** where a dragged panel would land */
export interface DropSpot {
  zone: ZoneName;
  /** index of the stack, or the stack count to add one at the end */
  stack: number;
  /** into an existing stack as a tab, or as a new stack before or after it */
  place: "into" | "before" | "after";
}

export interface PanelPlace {
  zone: ZoneName;
  stack: number;
  index: number;
}

export function emptyZone(size: number, open = true): DockZone {
  return { size, open, stacks: [] };
}

export function stack(panels: string[], weight = 1): PanelStack {
  return { panels, active: panels[0] ?? null, weight };
}

function copy(layout: DockLayout): DockLayout {
  return structuredClone(layout);
}

export function findPanel(layout: DockLayout, id: string): PanelPlace | null {
  for (const zone of ZONES) {
    const stacks = layout[zone].stacks;
    for (let at = 0; at < stacks.length; at++) {
      const index = stacks[at]?.panels.indexOf(id) ?? -1;
      if (index >= 0) return { zone, stack: at, index };
    }
  }
  return null;
}

/** every panel in the layout, in zone order */
export function panelsIn(layout: DockLayout): string[] {
  return ZONES.flatMap((zone) => layout[zone].stacks.flatMap((stack) => stack.panels));
}

/** drops empty stacks and makes sure each stack's active panel is one of its own */
function tidy(layout: DockLayout): DockLayout {
  for (const zone of ZONES) {
    const target = layout[zone];
    target.stacks = target.stacks.filter((stack) => stack.panels.length > 0);
    for (const stack of target.stacks) {
      if (!stack.active || !stack.panels.includes(stack.active)) {
        stack.active = stack.panels[0] ?? null;
      }
    }
  }
  return layout;
}

export function removePanel(layout: DockLayout, id: string): DockLayout {
  const next = copy(layout);
  for (const zone of ZONES) {
    for (const stack of next[zone].stacks) {
      const index = stack.panels.indexOf(id);
      if (index < 0) continue;
      stack.panels.splice(index, 1);
      // the tab next to it takes over, as with document tabs
      if (stack.active === id) {
        stack.active = stack.panels[Math.min(index, stack.panels.length - 1)] ?? null;
      }
    }
  }
  return tidy(next);
}

/** moves a panel to a drop spot. dropping a panel onto its own lone stack changes nothing */
export function movePanel(layout: DockLayout, id: string, spot: DropSpot): DockLayout {
  const from = findPanel(layout, id);
  if (from && from.zone === spot.zone && from.stack === spot.stack) {
    const alone = layout[from.zone].stacks[from.stack]?.panels.length === 1;
    if (spot.place === "into" || alone) return activatePanel(layout, id);
  }

  // removing the panel can remove its stack, which shifts the target index
  const removedStack =
    from !== null &&
    from.zone === spot.zone &&
    from.stack < spot.stack &&
    layout[from.zone].stacks[from.stack]?.panels.length === 1;
  const next = from ? removePanel(layout, id) : copy(layout);
  const zone = next[spot.zone];
  const index = removedStack ? spot.stack - 1 : spot.stack;
  zone.open = true;

  const target = zone.stacks[index];
  if (spot.place === "into" && target) {
    target.panels.push(id);
    target.active = id;
  } else {
    const at = spot.place === "after" ? index + 1 : index;
    zone.stacks.splice(Math.min(at, zone.stacks.length), 0, stack([id]));
  }
  return tidy(next);
}

export function activatePanel(layout: DockLayout, id: string): DockLayout {
  const place = findPanel(layout, id);
  if (!place) return layout;
  const next = copy(layout);
  const target = next[place.zone].stacks[place.stack];
  if (target) target.active = id;
  next[place.zone].open = true;
  return next;
}

/**
 * shows a panel. one that is already docked comes to the front. one that is
 * not goes back where the default layout keeps it, or into `fallback`
 */
export function openPanel(
  layout: DockLayout,
  id: string,
  defaults: DockLayout,
  fallback: ZoneName = "right",
): DockLayout {
  if (findPanel(layout, id)) return activatePanel(layout, id);
  const home = findPanel(defaults, id);
  const zone = home?.zone ?? fallback;
  // join the stack holding its neighbours from the default layout, if there is one
  const neighbours = home ? (defaults[home.zone].stacks[home.stack]?.panels ?? []) : [];
  const together = layout[zone].stacks.findIndex((stack) =>
    stack.panels.some((panel) => neighbours.includes(panel)),
  );
  if (together >= 0) return movePanel(layout, id, { zone, stack: together, place: "into" });
  return movePanel(layout, id, { zone, stack: layout[zone].stacks.length, place: "before" });
}

export function togglePanel(layout: DockLayout, id: string, defaults: DockLayout): DockLayout {
  const place = findPanel(layout, id);
  if (!place) return openPanel(layout, id, defaults);
  const zone = layout[place.zone];
  const showing = zone.open && zone.stacks[place.stack]?.active === id;
  return showing ? removePanel(layout, id) : activatePanel(layout, id);
}

export function resizeZone(layout: DockLayout, zone: ZoneName, size: number): DockLayout {
  const next = copy(layout);
  const { min, max } = ZONE_LIMITS[zone];
  next[zone].size = Math.round(Math.min(max, Math.max(min, size)) * 4) / 4;
  return next;
}

/** shares space between two neighbouring stacks, keeping their combined weight */
export function shareStacks(
  layout: DockLayout,
  zone: ZoneName,
  first: number,
  fraction: number,
): DockLayout {
  const next = copy(layout);
  const a = next[zone].stacks[first];
  const b = next[zone].stacks[first + 1];
  if (!a || !b) return layout;
  const total = a.weight + b.weight;
  const share = Math.min(0.9, Math.max(0.1, fraction));
  a.weight = total * share;
  b.weight = total - a.weight;
  return next;
}

export function setZoneOpen(layout: DockLayout, zone: ZoneName, open: boolean): DockLayout {
  const next = copy(layout);
  next[zone].open = open;
  return next;
}

/** true when any zone with panels in it is showing */
export function anyZoneOpen(layout: DockLayout): boolean {
  return ZONES.some((zone) => layout[zone].open && layout[zone].stacks.length > 0);
}

/** hides every zone, or shows them all again */
export function setAllZones(layout: DockLayout, open: boolean): DockLayout {
  const next = copy(layout);
  for (const zone of ZONES) next[zone].open = open;
  return next;
}

/** drops panels this build does not know, such as ones from a plugin that is gone */
export function keepKnown(layout: DockLayout, known: ReadonlySet<string>): DockLayout {
  const next = copy(layout);
  for (const zone of ZONES) {
    for (const stack of next[zone].stacks) {
      stack.panels = stack.panels.filter((panel) => known.has(panel));
    }
  }
  return tidy(next);
}
