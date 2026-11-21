import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";

import type { EngineStatus } from "../../bindings/EngineStatus";
import { Button } from "../../components/Button";
import { Tooltip } from "../../components/Tooltip";
import { formatBytes } from "../../lib/format";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { useSettings } from "../../state/settings";

/** which device draws pictures, and why when it is not the graphics card */
export function RenderDevice() {
  const useGpu = useSettings((s) => s.settings?.render.useGpu);
  const [status, setStatus] = useState<EngineStatus | null>(null);
  useEffect(() => {
    if (!inDesktopApp) return;
    let stale = false;
    // the engine applies a changed setting on its own thread, so look again shortly after
    const timer = window.setTimeout(() => {
      void ipc.renderStatus().then((s) => {
        if (!stale) setStatus(s);
      });
    }, 300);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [useGpu]);

  if (!inDesktopApp) return <span className="text-xs text-fg-subtle">Desktop app only</span>;
  if (!status) return <span className="text-xs text-fg-subtle">Checking...</span>;
  const label =
    status.backend === "gpu"
      ? `${status.adapter ?? "Graphics card"}${status.software ? " (software)" : ""}`
      : "The processor";
  const detail = [
    status.note,
    `Results kept: ${formatBytes(status.gpuCacheBytes)} on the card, ${formatBytes(status.cpuCacheBytes)} in memory.`,
    status.recoveries > 0
      ? `The graphics card stopped responding ${String(status.recoveries)} time${status.recoveries === 1 ? "" : "s"} and Vellum reconnected.`
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <Tooltip text={detail}>
      <span className="max-w-64 truncate text-right text-xs text-fg-muted">{label}</span>
    </Tooltip>
  );
}

/** an icc profile for the monitor, picked from a file */
export function DisplayProfile() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  if (!settings) return null;
  const path = settings.render.displayProfile;
  return (
    <span className="flex max-w-80 items-center gap-2">
      <span className="min-w-0 truncate text-xs text-fg-muted" title={path ?? undefined}>
        {path ? (path.split(/[\\/]/).pop() ?? path) : "sRGB"}
      </span>
      <Button
        size="sm"
        tooltip="Pick the colour profile file of your monitor"
        icon={<FolderOpen className="size-3.5" />}
        disabled={!inDesktopApp}
        onClick={() => {
          void open({
            title: "Display colour profile",
            filters: [{ name: "Colour profiles", extensions: ["icc", "icm"] }],
          }).then((picked) => {
            if (typeof picked === "string") void update({ render: { displayProfile: picked } });
          });
        }}
      >
        Choose
      </Button>
      {path !== null && (
        <Button
          size="sm"
          variant="ghost"
          tooltip="Show colours as sRGB, right for most monitors"
          onClick={() => {
            void update({ render: { displayProfile: null } });
          }}
        >
          Reset
        </Button>
      )}
    </span>
  );
}
