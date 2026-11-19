// the dock layout: three docks (left, right, bottom) around the document, each holding
// groups of panels shown as tabs. every function here is pure and returns a new layout, so
// the shell can save the result straight to settings and the tests need no react.

import type { DockArea } from "../bindings/DockArea";
import type { DockGroup } from "../bindings/DockGroup";
import type { DockLayout } from "../bindings/DockLayout";

export type AreaName = "left" | "right" | "bottom";
export const areaNames: AreaName[] = ["left", "right", "bottom"];

/** where a dragged panel lands: as a tab in a group, or as a new group in a dock */
export type DockDrop =
  | { type: "tab"; area: AreaName; group: string; index: number }
  | { type: "group"; area: AreaName; index: number };

function clone(layout: DockLayout): DockLayout {
  return {
    left: cloneArea(layout.left),
    right: cloneArea(layout.right),
    bottom: cloneArea(layout.bottom),
  };
}

function cloneArea(area: DockArea): DockArea {
  return { ...area, groups: area.groups.map((g) => ({ ...g, panels: [...g.panels] })) };
}

/** drops unknown and repeated panels and empty groups, and fixes active tabs */
export function normalizeLayout(layout: DockLayout, known: readonly string[]): DockLayout {
  const next = clone(layout);
  const seen = new Set<string>();
  const ids = new Set<string>();
  for (const name of areaNames) {
    const area = next[name];
    area.groups = area.groups
      .map((group) => {
        const panels = group.panels.filter((p) => known.includes(p) && !seen.has(p));
        panels.forEach((p) => seen.add(p));
        const active = panels.includes(group.active) ? group.active : (panels[0] ?? "");
        return { ...group, panels, active };
      })
      .filter((group) => group.panels.length > 0);
    for (const group of area.groups) {
      if (!group.id || ids.has(group.id)) group.id = newGroupId(ids);
      ids.add(group.id);
    }
  }
  return next;
}

function newGroupId(taken: Set<string>): string {
  let n = taken.size + 1;
  while (taken.has(`group-${n}`)) n++;
  return `group-${n}`;
}

function allGroupIds(layout: DockLayout): Set<string> {
  return new Set(areaNames.flatMap((name) => layout[name].groups.map((g) => g.id)));
}

export interface PanelPlace {
  area: AreaName;
  group: DockGroup;
}

export function findPanel(layout: DockLayout, panel: string): PanelPlace | null {
  for (const area of areaNames) {
    const group = layout[area].groups.find((g) => g.panels.includes(panel));
    if (group) return { area, group };
  }
  return null;
}

/** true if the panel is placed and its dock is open */
export function isPanelVisible(layout: DockLayout, panel: string): boolean {
  const place = findPanel(layout, panel);
  return place !== null && !layout[place.area].collapsed && place.group.active === panel;
}

export function closePanel(layout: DockLayout, panel: string): DockLayout {
  const next = clone(layout);
  for (const area of areaNames) {
    next[area].groups = next[area].groups
      .map((g) => {
        const panels = g.panels.filter((p) => p !== panel);
        const active = g.active === panel ? (panels[0] ?? "") : g.active;
        return { ...g, panels, active };
      })
      .filter((g) => g.panels.length > 0);
  }
  return next;
}

/** brings a panel to the front, opening its dock, or adds it where it usually lives */
export function showPanel(layout: DockLayout, panel: string, home: AreaName): DockLayout {
  const next = clone(layout);
  const place = findPanel(next, panel);
  if (place) {
    place.group.active = panel;
    next[place.area].collapsed = false;
    return next;
  }
  const area = next[home];
  area.collapsed = false;
  const first = area.groups[0];
  if (first) {
    first.panels.push(panel);
    first.active = panel;
  } else {
    area.groups.push({
      id: newGroupId(allGroupIds(next)),
      panels: [panel],
      active: panel,
      weight: 1,
    });
  }
  return next;
}

/** shows the panel if it is hidden or behind another tab, hides its dock if it is in front */
export function togglePanel(layout: DockLayout, panel: string, home: AreaName): DockLayout {
  const place = findPanel(layout, panel);
  if (place && isPanelVisible(layout, panel)) {
    return setCollapsed(layout, place.area, true);
  }
  return showPanel(layout, panel, home);
}

export function setActive(layout: DockLayout, groupId: string, panel: string): DockLayout {
  const next = clone(layout);
  for (const area of areaNames) {
    const group = next[area].groups.find((g) => g.id === groupId);
    if (group?.panels.includes(panel)) group.active = panel;
  }
  return next;
}

export function movePanel(layout: DockLayout, panel: string, drop: DockDrop): DockLayout {
  const next = clone(layout);
  const from = findPanel(next, panel);
  if (!from) return layout;

  // leave a marker where the panel was, insert it at the target, then remove the marker.
  // indexes from the drop never need adjusting that way.
  const marker = "\u0000moving";
  const at = from.group.panels.indexOf(panel);
  from.group.panels[at] = marker;

  const target = next[drop.area];
  if (drop.type === "tab") {
    const group = target.groups.find((g) => g.id === drop.group);
    if (!group) return layout;
    group.panels.splice(Math.max(0, Math.min(drop.index, group.panels.length)), 0, panel);
    group.active = panel;
  } else {
    const fresh: DockGroup = {
      id: newGroupId(allGroupIds(next)),
      panels: [panel],
      active: panel,
      weight: 1,
    };
    target.groups.splice(Math.max(0, Math.min(drop.index, target.groups.length)), 0, fresh);
  }
  target.collapsed = false;

  from.group.panels = from.group.panels.filter((p) => p !== marker);
  if (from.group.active === panel && !from.group.panels.includes(panel)) {
    from.group.active = from.group.panels[Math.min(at, from.group.panels.length - 1)] ?? "";
  }
  next[from.area].groups = next[from.area].groups.filter((g) => g.panels.length > 0);
  return next;
}

export function setCollapsed(layout: DockLayout, area: AreaName, collapsed: boolean): DockLayout {
  const next = clone(layout);
  next[area].collapsed = collapsed;
  return next;
}

export function resizeArea(layout: DockLayout, area: AreaName, size: number): DockLayout {
  const [min, max] = area === "bottom" ? [100, 600] : [160, 640];
  const next = clone(layout);
  next[area].size = Math.round(Math.min(max, Math.max(min, size)));
  return next;
}

/** sets the share of one group and the one after it, as when dragging the line between */
export function resizeGroups(
  layout: DockLayout,
  area: AreaName,
  index: number,
  weights: [number, number],
): DockLayout {
  const next = clone(layout);
  const a = next[area].groups[index];
  const b = next[area].groups[index + 1];
  if (!a || !b) return layout;
  a.weight = Math.min(10, Math.max(0.1, weights[0]));
  b.weight = Math.min(10, Math.max(0.1, weights[1]));
  return next;
}

/** true if any dock with panels is open */
export function anyDockOpen(layout: DockLayout): boolean {
  return areaNames.some((name) => !layout[name].collapsed && layout[name].groups.length > 0);
}

/** hides every dock, or shows every dock that has panels when they are all hidden */
export function toggleDocks(layout: DockLayout): DockLayout {
  const open = anyDockOpen(layout);
  const next = clone(layout);
  for (const name of areaNames) {
    next[name].collapsed = open ? true : next[name].groups.length === 0;
  }
  return next;
}

function area(size: number, groups: DockGroup[], collapsed = false): DockArea {
  return { size, collapsed, groups };
}

function group(id: string, panels: string[]): DockGroup {
  return { id, panels, active: panels[0] ?? "", weight: 1 };
}

export interface LayoutPreset {
  id: string;
  name: string;
  description: string;
  layout: DockLayout;
}

/** built in layouts. "default" matches DockLayout::default() in rust, checked by a test. */
export const layoutPresets: LayoutPreset[] = [
  {
    id: "default",
    name: "Default",
    description: "Info and history on the right, jobs below them.",
    layout: {
      left: area(260, []),
      right: area(300, [group("right-top", ["info", "history"]), group("right-bottom", ["jobs"])]),
      bottom: area(220, []),
    },
  },
  {
    id: "editing",
    name: "Editing",
    description: "History on the left, info on the right, jobs along the bottom.",
    layout: {
      left: area(240, [group("left", ["history"])]),
      right: area(280, [group("right", ["info"])]),
      bottom: area(180, [group("bottom", ["jobs"])]),
    },
  },
  {
    id: "focus",
    name: "Focus",
    description: "Every panel tucked away. Bring them back with the panel buttons.",
    layout: {
      left: area(260, []),
      right: area(
        300,
        [group("right-top", ["info", "history"]), group("right-bottom", ["jobs"])],
        true,
      ),
      bottom: area(220, []),
    },
  },
];
