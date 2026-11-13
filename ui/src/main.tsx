import "./styles/index.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { desktopBackend } from "./lib/backend";
import { connect } from "./lib/connect";

const root = document.getElementById("root");
if (!root) throw new Error("index.html is missing its #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

void connect(desktopBackend());
