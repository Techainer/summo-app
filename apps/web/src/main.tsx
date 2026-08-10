import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { EngineProvider } from "./lib/engine-context";
import { router } from "./router";
import "./styles/theme.css";
// Screens still to be migrated off the hand-written stylesheet. Removed when the last one is done.
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

// The engine sits above the router: navigating between screens must not interrupt a recording.
createRoot(root).render(
  <StrictMode>
    <EngineProvider>
      <RouterProvider router={router} />
    </EngineProvider>
  </StrictMode>,
);
