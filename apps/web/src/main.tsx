import { RouterProvider } from "@tanstack/react-router";
import { LazyMotion, MotionConfig } from "motion/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { AppI18n } from "./i18n/AppI18n";
import { REDUCED } from "./lib/motion";
import { EngineProvider } from "./lib/engine-provider";
import { router, warmScreens } from "./router";
import "./styles/theme.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

/**
 * The other screens, once this one is on the glass.
 *
 * They are separate chunks so that opening the app does not parse the settings form and the models
 * catalogue first. Left at that, the cost would only have moved: the first visit to each screen
 * would pay a fetch at the exact moment somebody is waiting for it. Idle time is when neither is
 * true — the first screen has rendered and nothing else wants the main thread.
 *
 * `requestIdleCallback` where it exists, which is everywhere except Safari and the WebKit view the
 * macOS and iOS shells use; there a timeout after first paint is the same idea with a worse clock.
 */
// `typeof`, not `"requestIdleCallback" in window`: the DOM types declare it unconditionally, so
// the `in` check narrows the else branch to `never` and the fallback stops type-checking. What is
// being asked here is whether this browser actually has it, which is a runtime question.
const warm = () => setTimeout(warmScreens, 200);
if (typeof requestIdleCallback === "function") requestIdleCallback(warm, { timeout: 3000 });
else window.addEventListener("load", warm, { once: true });

// The engine sits above the router: navigating between screens must not interrupt a recording.
createRoot(root).render(
  <StrictMode>
    {/* Reduced motion is honoured here, once, rather than in each component. Motion keeps opacity
        and drops transforms — so a cross-fade still says something changed, while the sliding and
        scaling that cause discomfort go away. Vestibular disorders make large sliding movement
        genuinely unpleasant; an app that ignores the setting is unusable for those people. */}
    <MotionConfig reducedMotion={REDUCED}>
      {/* The animation engine, fetched beside the first screen rather than inside it.

          Every animated element in the app is an `m.div` rather than a `motion.div`, and the
          difference is where the engine lives: `motion.div` carries keyframes, springs, gestures
          and value interpolation into whichever chunk imports it, which was the entry chunk,
          because the shell and the home screen animate. `m` is the same component with no engine
          attached, and `LazyMotion` supplies one to all of them at once.

          `strict` is what keeps it true: a `motion.div` added later throws instead of quietly
          putting 34 kB back into the first load, which is exactly how it got there the first time.

          Until the bundle arrives, an `m` element renders at its `initial` state and animates when
          it lands — a frame or two later from a local daemon. That is also the failure mode worth
          naming: if this chunk never arrives, elements that start at `opacity: 0` stay there. It
          is served from the same bundle as the code asking for it, so the case where it fails
          alone is the case where the app is already broken. */}
      <LazyMotion strict features={() => import("./lib/motion-features").then((f) => f.default)}>
        <EngineProvider>
          <AppI18n>
            <RouterProvider router={router} />
          </AppI18n>
        </EngineProvider>
      </LazyMotion>
    </MotionConfig>
  </StrictMode>,
);
