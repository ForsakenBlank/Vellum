import { audioWorkspace } from "./audio";
import { colorWorkspace } from "./color";
import { imageWorkspace } from "./image";
import { motionWorkspace } from "./motion";
import { quickToolsWorkspace } from "./quick-tools";
import { vectorWorkspace } from "./vector";
import { videoWorkspace } from "./video";
import type { Workspace } from "./types";

export type { Workspace } from "./types";

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
