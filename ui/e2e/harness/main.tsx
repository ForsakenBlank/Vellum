// the app running in a plain browser against an in memory backend, for
// playwright tests and for looking at the ui without building the desktop app.
// it is not part of the production build
import "../../src/styles/index.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "../../src/App";
import { connect } from "../../src/lib/connect";
import { MemoryBackend } from "./memoryBackend";

const backend = new MemoryBackend();
window.vellumHarness = backend;

const root = document.getElementById("root");
if (!root) throw new Error("index.html is missing its #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

void connect(backend);
