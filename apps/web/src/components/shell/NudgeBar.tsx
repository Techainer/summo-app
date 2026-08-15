import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, m } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { GENTLE, collapse } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import { NudgeClient, POLL_MS, iconFor, notify, type Nudge, type Reason } from "../../lib/nudges";

/**
 * Which one speaks first, when several are due at once.
 *
 * A meeting about to start is the only one with a deadline — the prompt is worth nothing a minute
 * later — so it goes above a draft that has been waiting since yesterday and an overdue task that
 * will still be overdue tomorrow. The two summaries are the least urgent things the daemon says,
 * and they are the two most likely to arrive together.
 */
const ORDER: Reason[] = [
  "meeting-soon",
  "draft-waiting",
  "overdue",
  "weekly-rollup",
  "daily-report",
];

/**
 * What the agent wants to tell you, when it is allowed to.
 *
 * Shown in the app as a strip under the header, and as an OS notification when the user has
 * allowed those. Both, not either: a notification the user missed while in another app should
 * still be waiting when they come back, and the daemon only hands each one out once.
 *
 * ## One at a time
 *
 * Each nudge used to draw its own full-width bar, and nothing bounded how many. A Monday morning
 * with a draft waiting, three overdue tasks and a weekly summary opened the app with three of them
 * stacked above the content — 150 px of notice on a 900 px window, and worse on a laptop. Notice
 * that large stops being read; it becomes a header with a close button.
 *
 * So the strip shows the most urgent one and says how many are behind it. Dismissing brings the
 * next forward, and the count is a button for a person who would rather see them all at once.
 * Nothing is hidden and nothing is dropped — the queue is the same queue.
 *
 * Dismissing removes it from the strip and nothing else. The underlying thing — an unread draft,
 * an overdue task — is still there, and the daemon will not repeat itself today.
 */
export function NudgeBar() {
  const { handshake, session, start } = useEngine();
  const client = useMemo(() => new NudgeClient(handshake), [handshake]);
  const navigate = useNavigate();
  const [queue, setQueue] = useState<Nudge[]>([]);
  const [expanded, setExpanded] = useState(false);
  const t = useT();

  const go = useCallback(
    (route: string) => {
      void navigate({ to: route });
    },
    [navigate],
  );

  const poll = useCallback(async () => {
    try {
      const due = await client.due();
      if (due.length === 0) return;
      for (const nudge of due) notify(nudge, go);
      setQueue((current) => [...current, ...due]);
    } catch {
      // The daemon being briefly unreachable is not worth telling anyone about.
    }
  }, [client, go]);

  useEffect(() => {
    // Not while recording: the daemon already refuses, and asking would waste a round trip at the
    // one moment the machine is busiest.
    if (session.recording) return undefined;

    // The first poll goes through the same timer as the rest, at zero delay, rather than being
    // called straight from the effect body. Same behaviour — a request as soon as the bar mounts —
    // without a state write that lands before the browser has painted the bar it belongs to.
    const first = window.setTimeout(() => void poll(), 0);
    const timer = window.setInterval(() => void poll(), POLL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(timer);
    };
  }, [poll, session.recording]);

  const dismiss = (key: string) => setQueue((q) => q.filter((n) => n.key !== key));

  // Sorted for display only. The queue keeps arrival order, because that is what the daemon handed
  // out and what a second poll appends to.
  const ranked = useMemo(
    () => [...queue].sort((a, b) => ORDER.indexOf(a.reason) - ORDER.indexOf(b.reason)),
    [queue],
  );
  // Derived rather than stored: dismissing the second-to-last nudge should collapse the strip on
  // its own, and an effect that noticed and called `setExpanded(false)` would be a second render
  // for something the first one already knows.
  const showAll = expanded && ranked.length > 1;
  const shown = showAll ? ranked : ranked.slice(0, 1);
  const waiting = ranked.length - shown.length;

  return (
    <AnimatePresence initial={false}>
      {shown.map((nudge) => (
        <m.div
          key={nudge.key}
          variants={collapse}
          initial="hidden"
          animate="shown"
          exit="gone"
          transition={GENTLE}
          className="border-accent/25 bg-accent-soft overflow-hidden border-b"
        >
          <div className="flex items-center gap-3 px-4 py-2">
            <span aria-hidden="true" className="text-accent">
              {iconFor(nudge.reason)}
            </span>
            <p className="text-meta min-w-0 flex-1">
              <strong className="font-medium">{nudge.title}</strong>
              <span className="text-fg-dim"> — {nudge.body}</span>
            </p>
            {/* How many are behind this one, and a way to see them. Only on the first bar: an
                expanded strip is already showing everything, so a count there would be a button
                that says nothing. */}
            {waiting > 0 && (
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="text-fg-dim hover:text-fg hover:bg-fg/5 text-micro rounded-full px-2 py-1"
              >
                {t("nudge.more", { count: waiting })}
              </button>
            )}
            {/* A meeting starting is the one nudge with an obvious next action, and making the
                user navigate to the record button to take it would waste the minute the prompt
                exists to save. Recording still begins because a person pressed something. */}
            <button
              type="button"
              onClick={() => {
                if (nudge.reason === "meeting-soon") {
                  void start();
                } else {
                  go(nudge.route);
                }
                dismiss(nudge.key);
              }}
              className="text-accent hover:bg-accent/10 text-meta rounded-full px-2.5 py-1 font-medium"
            >
              {nudge.reason === "meeting-soon" ? t("nudge.record") : t("nudge.view")}
            </button>
            <button
              type="button"
              onClick={() => dismiss(nudge.key)}
              aria-label={t("nudge.dismiss", { title: nudge.title })}
              className="text-fg-faint hover:text-fg rounded-lg px-2 py-1"
            >
              ✕
            </button>
          </div>
        </m.div>
      ))}
    </AnimatePresence>
  );
}
