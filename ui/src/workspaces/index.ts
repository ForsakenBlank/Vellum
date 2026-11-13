import { audioWorkspace } from "./audio";
import { colorWorkspace } from "./color";
import { homeWorkspace } from "./home";
import { imageWorkspace } from "./image";
import { motionWorkspace } from "./motion";
import { quickToolsWorkspace } from "./quick-tools";
import type { Workspace } from "./types";
import { vectorWorkspace } from "./vector";
import { videoWorkspace } from "./video";

export type { PanelDef, Workspace } from "./types";
export { homeWorkspace };

/** every workspace, in the order the rail shows them */
export const workspaces: Workspace[] = [
  quickToolsWorkspace,
  imageWorkspace,
  videoWorkspace,
  audioWorkspace,
  colorWorkspace,
  motionWorkspace,
  vectorWorkspace,
];

export function workspaceById(id: string | null): Workspace {
  if (id === null) return homeWorkspace;
  return workspaces.find((workspace) => workspace.id === id) ?? homeWorkspace;
}
