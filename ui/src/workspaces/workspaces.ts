/** the workspaces from PLAN.md section 4, with the phase that builds each one */
export interface WorkspaceInfo {
  id: string;
  label: string;
  phase: number;
}

export const workspaces: WorkspaceInfo[] = [
  { id: "quick-tools", label: "Quick tools", phase: 3 },
  { id: "image", label: "Image", phase: 5 },
  { id: "video", label: "Video", phase: 8 },
  { id: "color", label: "Colour", phase: 10 },
  { id: "audio", label: "Audio", phase: 11 },
  { id: "motion", label: "Motion", phase: 12 },
  { id: "vector", label: "Vector", phase: 15 },
];
