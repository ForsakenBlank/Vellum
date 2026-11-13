import { create } from "zustand";

import type { CommandSpec } from "../bindings/CommandSpec";

/** the commands the backend registered, loaded once at startup */
interface CommandsState {
  backend: CommandSpec[];
  apply: (specs: CommandSpec[]) => void;
}

export const useBackendCommands = create<CommandsState>()((set) => ({
  backend: [],
  apply: (backend) => {
    set({ backend });
  },
}));
