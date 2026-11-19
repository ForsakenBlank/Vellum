import { Search } from "lucide-react";

import { Kbd } from "../components/Kbd";
import { Tooltip } from "../components/Tooltip";
import { runCommand, useCommands } from "../state/commands";
import { DocumentTabs } from "./DocumentTabs";
import { MenuBar } from "./MenuBar";
import { NotificationBell } from "./NotificationCenter";

/** the top bar: logo, menus, tabs, the palette button and the notification bell */
export function TitleBar() {
  const chord = useCommands((s) => s.keymap?.bindings["app.command_palette"]?.[0]);
  return (
    <header className="flex h-(--titlebar) shrink-0 items-center gap-2 border-b border-line bg-panel pr-2 pl-2.5">
      <div className="flex items-center gap-2 pr-1" aria-label="Vellum">
        <Logo />
      </div>
      <MenuBar />
      <span className="mx-1 h-4 w-px bg-line" />
      <DocumentTabs />
      <Tooltip text="Search every command, setting and recent file" command="app.command_palette">
        <button
          type="button"
          aria-label="Command palette"
          onClick={() => void runCommand("app.command_palette")}
          className="flex h-7 w-60 shrink-0 items-center gap-2 rounded-md border border-line bg-canvas/60 px-2 text-xs text-fg-subtle transition-colors duration-(--motion-fast) hover:border-line-strong hover:text-fg-muted max-lg:w-auto"
        >
          <Search className="size-3.5" />
          <span className="flex-1 truncate text-left whitespace-nowrap max-lg:hidden">
            Search or run a command
          </span>
          {chord && <Kbd chord={chord} />}
        </button>
      </Tooltip>
      <NotificationBell />
    </header>
  );
}

/** the mark: a sheet of vellum, a translucent square laid over another */
export function Logo({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden className="shrink-0">
      <rect x="2" y="5" width="11" height="13" rx="2.5" fill="var(--accent)" />
      <rect
        x="7"
        y="2"
        width="11"
        height="13"
        rx="2.5"
        fill="var(--fg-1)"
        fillOpacity="0.16"
        stroke="var(--fg-1)"
        strokeOpacity="0.55"
        strokeWidth="1.2"
      />
    </svg>
  );
}
