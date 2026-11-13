import { ListChecks, type LucideIcon } from "lucide-react";

import type { DockLayout } from "../bindings/DockLayout";
import { EmptyState } from "../components/EmptyState";
import { JobsPanel } from "../shell/JobsPanel";
import type { PanelDef } from "../workspaces/types";
import { emptyZone, stack } from "./layout";

/** a panel that holds the place of one a later phase builds */
export function placeholderPanel(
  id: string,
  title: string,
  icon: LucideIcon,
  arrives: string,
): PanelDef {
  function Placeholder() {
    return <EmptyState icon={icon} title={title} body={arrives} compact />;
  }
  Placeholder.displayName = `${title}Placeholder`;
  return { id, title, icon, component: Placeholder };
}

export const jobsPanel: PanelDef = {
  id: "jobs",
  title: "Jobs",
  icon: ListChecks,
  component: JobsPanel,
};

type Stacks = string[][];

/** builds a layout from lists of panel ids, one list per stack */
export function dockLayout(zones: {
  left?: Stacks;
  right?: Stacks;
  bottom?: Stacks;
  sizes?: { left?: number; right?: number; bottom?: number };
}): DockLayout {
  const zone = (stacks: Stacks | undefined, size: number) => ({
    ...emptyZone(size),
    stacks: (stacks ?? []).map((panels) => stack(panels)),
  });
  return {
    left: zone(zones.left, zones.sizes?.left ?? 16),
    right: zone(zones.right, zones.sizes?.right ?? 18),
    bottom: zone(zones.bottom, zones.sizes?.bottom ?? 14),
  };
}
