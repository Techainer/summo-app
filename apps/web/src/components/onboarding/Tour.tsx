import { useRouterState } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";

import { useI18n } from "../../i18n/context";
import { Button } from "../ui";

/**
 * A short tour, once.
 *
 * Deliberately not a spotlight overlay pointing at elements. Those break the moment a layout
 * changes or a screen is narrow, they trap keyboard focus, and they cannot be read by anyone using
 * a screen reader. This is four cards in the corner that say what the app does and get out of the
 * way — the app is usable underneath them the whole time.
 *
 * It is dismissible at every step and never comes back. A tutorial that reappears is a tutorial the
 * user learns to close without reading.
 *
 * It only ever appears on the record screen. That is correctness, not taste: a card pinned to the
 * bottom right corner sits on top of whatever is in that corner, and on the Ask screen that corner
 * holds the button you press to ask. The record screen's corner is empty, it is where the app opens,
 * and confining the tour to it is the difference between an overlay and an obstruction. Navigate
 * away and it steps aside; come back and it is where you left it.
 */

const STORAGE_KEY = "summo.tour";

/** The only route the tour shows on. See the note above. */
const HOME = "/";

/** The four things a new user has to know, in the order they will need them. */
export const STEPS = ["record", "summary", "tasks", "ask"] as const;
export type Step = (typeof STEPS)[number];

/** Whether the tour has been seen. Local, like the language choice: it is about this screen. */
export function seen(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "done";
  } catch {
    return true; // Storage locked down: better to skip a tour than to show it every launch.
  }
}

export function markSeen(): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, "done");
  } catch {
    // Nothing to do; the tour will offer itself again next launch.
  }
}

export function Tour({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [index, setIndex] = useState(0);
  const step = STEPS[index] ?? STEPS[0];
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const finish = () => {
    markSeen();
    onClose();
  };

  // Escape closes it, because anything that covers part of the screen has to.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const last = index === STEPS.length - 1;

  // Not marked as seen: somebody who navigated away has not read it, and it should be waiting when
  // they come back rather than silently spent.
  if (pathname !== HOME) return null;

  return (
    <AnimatePresence>
      <motion.aside
        // A polite live region rather than a dialog: it does not take focus, so somebody typing in
        // the app is not interrupted by it.
        role="complementary"
        aria-label={t("tour.title")}
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 12 }}
        className="fixed bottom-4 right-4 z-40 w-[min(22rem,calc(100vw-2rem))] rounded-2xl border border-line bg-bg-soft p-4 shadow-lg"
      >
        <p className="text-[12px] uppercase tracking-wide text-fg-faint">
          {t("tour.step", { current: index + 1, total: STEPS.length })}
        </p>
        <h2 className="mt-1 font-medium">{t(`tour.${step}_title`)}</h2>
        <p className="mt-1 text-sm text-fg-dim">{t(`tour.${step}_body`)}</p>

        <div className="mt-4 flex items-center gap-2">
          <Button
            onClick={() => (last ? finish() : setIndex((i) => i + 1))}
            className="flex-1"
          >
            {last ? t("tour.done") : t("tour.next")}
          </Button>
          <Button variant="ghost" onClick={finish}>
            {t("tour.skip")}
          </Button>
        </div>
      </motion.aside>
    </AnimatePresence>
  );
}
