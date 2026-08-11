import { RouterProvider } from "@tanstack/react-router";
import { MotionConfig } from "motion/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { AppI18n } from "./i18n/AppI18n";
import { REDUCED } from "./lib/motion";
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
    {/* Reduced motion is honoured here, once, rather than in each component. Motion keeps opacity
        and drops transforms — so a cross-fade still says something changed, while the sliding and
        scaling that cause discomfort go away. Vestibular disorders make large sliding movement
        genuinely unpleasant; an app that ignores the setting is unusable for those people. */}
    <MotionConfig reducedMotion={REDUCED}>
      <EngineProvider>
        <AppI18n>
          <RouterProvider router={router} />
        </AppI18n>
      </EngineProvider>
    </MotionConfig>
  </StrictMode>,
);
