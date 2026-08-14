import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { GENTLE, collapse } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import { NudgeClient, POLL_MS, iconFor, notify, type Nudge } from "../../lib/nudges";

/**
 * What the agent wants to tell you, when it is allowed to.
 *
 * Shown in the app as a strip under the header, and as an OS notification when the user has
 * allowed those. Both, not either: a notification the user missed while in another app should
 * still be waiting when they come back, and the daemon only hands each one out once.
 *
 * Dismissing removes it from the strip and nothing else. The underlying thing — an unread draft,
 * an overdue task — is still there, and the daemon will not repeat itself today.
 */
export function NudgeBar() {
  const { handshake, session, start } = useEngine();
  const client = useMemo(() => new NudgeClient(handshake), [handshake]);
  const navigate = useNavigate();
  const [queue, setQueue] = useState<Nudge[]>([]);
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

  return (
    <AnimatePresence initial={false}>
      {queue.map((nudge) => (
        <motion.div
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
        </motion.div>
      ))}
    </AnimatePresence>
  );
}
