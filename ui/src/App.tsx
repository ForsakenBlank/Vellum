import { TooltipProvider } from "./components/Tooltip";
import { AppShell } from "./shell/AppShell";

export function App() {
  return (
    <TooltipProvider delayDuration={400}>
      <AppShell />
    </TooltipProvider>
  );
}
