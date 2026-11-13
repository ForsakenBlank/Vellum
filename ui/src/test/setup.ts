import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import { useDock } from "../dock/state";
import { useDrop } from "../shell/drop";
import { useApp } from "../stores/app";
import { useBackendCommands } from "../stores/commands";
import { useJobs } from "../stores/jobs";
import { useNotifications } from "../stores/notifications";
import { useRecent } from "../stores/recent";
import { useSession } from "../stores/session";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { disconnectAll } from "./fakes";

const stores = [
  useApp,
  useJobs,
  useSettings,
  useSession,
  useRecent,
  useBackendCommands,
  useNotifications,
  useUi,
  useDock,
  useDrop,
] as const;

const initial = stores.map((store) => store.getState());

// stores are module level, so every test starts from a clean copy
afterEach(() => {
  cleanup();
  disconnectAll();
  stores.forEach((store, index) => {
    // each store gets back exactly the state it started with
    (store.setState as (state: unknown, replace: true) => void)(initial[index], true);
  });
  localStorage.clear();
});

// jsdom has no layout, so these are stubbed where components call them
Element.prototype.scrollIntoView = function scrollIntoView() {
  return undefined;
};
if (!("elementFromPoint" in document)) {
  Object.defineProperty(document, "elementFromPoint", { value: () => null });
}
// jsdom has no pointer capture, and the dock and tabs ask for it
Element.prototype.setPointerCapture = function setPointerCapture() {
  return undefined;
};
Element.prototype.releasePointerCapture = function releasePointerCapture() {
  return undefined;
};
