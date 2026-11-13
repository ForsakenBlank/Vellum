import { Tooltip } from "radix-ui";

import { AppShell } from "./shell/AppShell";

export function App() {
  return (
    <Tooltip.Provider delayDuration={400} skipDelayDuration={200}>
      <AppShell />
    </Tooltip.Provider>
  );
}
