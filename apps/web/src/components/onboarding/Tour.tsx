import { useRouterState } from "@tanstack/react-router";
import { AnimatePresence, m } from "motion/react";
import { useEffect, useState } from "react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { GENTLE } from "../../lib/motion";
import { STEPS, markSeen, type Step } from "../../lib/tour";
import { Button, Sticker, type StickerName } from "../ui";

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

/** The only route the tour shows on. See the note above. */
const HOME = "/";

/**
 * A drawing per step, so the four cards are visibly four things rather than one card whose words
 * changed.
 *
 * A person clicking "next" four times has no other signal that they moved: the card is the same
 * size in the same corner with a paragraph in the same place, and the step counter is six
 * characters of grey text. The picture is what makes the change legible at a glance.
 */
const DRAWING: Record<Step, StickerName> = {
  record: "sprout",
  summary: "robot",
  tasks: "pencil",
  ask: "magnifier",
};

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
      <m.aside
        // A polite live region rather than a dialog: it does not take focus, so somebody typing in
        // the app is not interrupted by it.
        role="complementary"
        aria-label={t("tour.title")}
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 12 }}
        // Above the bottom bar on a phone, beside it on a desktop. At 390px the card sat over the
        // navigation — the four things this tour is pointing at were behind the card explaining
        // them, and the first step tells you to press a button it was covering.
        className="border-line bg-bg-raised fixed right-4 bottom-[calc(env(safe-area-inset-bottom)+4.5rem)] left-4 z-40 rounded-[var(--radius-panel)] border p-4 shadow-[var(--shadow-pop)] sm:bottom-4 sm:left-auto sm:w-[min(22rem,calc(100vw-2rem))]"
      >
        <div className="flex items-start gap-3">
          {/* Keyed on the step, so moving on is a new drawing rather than the same one with
              different words beside it. */}
          <Sticker key={step} name={DRAWING[step]} size={64} className="-mt-1 -ml-1" />
          <div className="min-w-0 flex-1">
            <p className="text-fg-faint text-micro tracking-wide uppercase">
              {t("tour.step", { current: index + 1, total: STEPS.length })}
            </p>
            <h2 className="mt-1 font-semibold tracking-tight">{t(`tour.${step}_title`)}</h2>
          </div>
        </div>

        <m.p
          key={`${step}-body`}
          initial={{ opacity: 0, y: 4 }}
          animate={{ opacity: 1, y: 0 }}
          transition={GENTLE}
          className="text-fg-dim mt-2 text-sm text-pretty"
        >
          {t(`tour.${step}_body`)}
        </m.p>

        <div className="mt-4 flex items-center gap-3">
          {/* Four dots rather than only "2/4": which one you are on and how many are left is a
              shape, and a shape is read without being parsed. */}
          <ol aria-hidden="true" className="flex items-center gap-1.5">
            {STEPS.map((each, at) => (
              <li
                key={each}
                className={cn(
                  "h-1.5 rounded-full transition-all duration-300",
                  at === index
                    ? "bg-accent w-5"
                    : at < index
                      ? "bg-accent/40 w-1.5"
                      : "bg-line w-1.5",
                )}
              />
            ))}
          </ol>
          {/* The action that moves forward stays the filled one on every step, including the last:
              "Xong" is what finishing the tour is called, not a way out of it. Skipping keeps its
              own quieter button until there is nothing left to skip. */}
          <div className="ml-auto flex items-center gap-2">
            {!last && (
              <Button variant="ghost" onClick={finish}>
                {t("tour.skip")}
              </Button>
            )}
            <Button onClick={() => (last ? finish() : setIndex((i) => i + 1))}>
              {last ? t("tour.done") : t("tour.next")}
            </Button>
          </div>
        </div>
      </m.aside>
    </AnimatePresence>
  );
}
