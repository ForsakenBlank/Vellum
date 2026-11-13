import { AudioLines, AudioWaveform, Gauge, History, SlidersVertical } from "lucide-react";

import { dockLayout, jobsPanel, placeholderPanel } from "../../dock/panels";
import type { Workspace } from "../types";

export const audioWorkspace: Workspace = {
  id: "audio",
  title: "Audio",
  icon: AudioLines,
  summary: "Waveform and spectral editing, effects and a mixer.",
  phase: 11,
  accepts: "audio and videos",
  panels: [
    placeholderPanel("effects-rack", "Effects", AudioWaveform, "Effects on the selected track."),
    placeholderPanel("mixer", "Mixer", SlidersVertical, "Levels, panning and buses."),
    placeholderPanel("meters", "Meters", Gauge, "Loudness and peak meters."),
    placeholderPanel("history", "History", History, "Every edit, with snapshots."),
    jobsPanel,
  ],
  layout: dockLayout({
    right: [["effects-rack", "history"]],
    bottom: [["mixer", "meters"]],
  }),
};
