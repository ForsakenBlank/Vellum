import { useGlobalShortcuts } from "../commands/useShortcuts";
import { Skeleton } from "../components/Skeleton";
import { Dock } from "../dock/Dock";
import { useApp } from "../stores/app";
import { useActiveDocument, useSession, useView, useWorkspaceId } from "../stores/session";
import { homeWorkspace, workspaceById } from "../workspaces";
import { DropOverlay } from "./DropOverlay";
import { Home } from "./Home";
import { Notice } from "./Notice";
import { Toasts } from "./Notifications";
import { Palette } from "./Palette";
import { Rail } from "./Rail";
import { RecoveryPrompt } from "./RecoveryPrompt";
import { SettingsDialog } from "./settings/SettingsDialog";
import { TopBar } from "./TopBar";
import { WorkspaceView } from "./WorkspaceView";

export function AppShell() {
  useGlobalShortcuts();
  const status = useApp((state) => state.status);
  const loaded = useSession((state) => state.session !== null);
  const view = useView();
  const workspace = workspaceById(useWorkspaceId());
  const document = useActiveDocument();

  return (
    <div className="flex h-full flex-col bg-canvas text-ink">
      <TopBar workspace={workspace} />
      <Notice />
      <div className="flex min-h-0 flex-1">
        <Rail />
        <main className="min-w-0 flex-1">
          {status === "connecting" && !loaded ? (
            <StartingUp />
          ) : view.type === "home" ? (
            <Dock workspace={homeWorkspace}>
              <Home />
            </Dock>
          ) : (
            <WorkspaceView workspace={workspace} document={document} />
          )}
        </main>
      </div>
      <DropOverlay />
      <Palette />
      <SettingsDialog />
      <RecoveryPrompt />
      <Toasts />
    </div>
  );
}

/** the shape of the home screen while the backend answers, so nothing jumps */
function StartingUp() {
  return (
    <div
      aria-label="Starting up"
      aria-busy
      className="mx-auto flex max-w-5xl flex-col gap-9 px-8 pt-10"
    >
      <div className="flex flex-col gap-2">
        <Skeleton className="h-7 w-40" />
        <Skeleton className="h-4 w-72" />
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
        {[0, 1, 2, 3, 4].map((index) => (
          <Skeleton key={index} className="h-32 rounded-xl" />
        ))}
      </div>
    </div>
  );
}
