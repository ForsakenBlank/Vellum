import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import { useApp } from "../stores/app";
import { useJobs } from "../stores/jobs";
import { useSettings } from "../stores/settings";

const initial = {
  app: useApp.getState(),
  jobs: useJobs.getState(),
  settings: useSettings.getState(),
};

// stores are module level, so every test starts from a clean copy
afterEach(() => {
  cleanup();
  useApp.setState(initial.app, true);
  useJobs.setState(initial.jobs, true);
  useSettings.setState(initial.settings, true);
});
