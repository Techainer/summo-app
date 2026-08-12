import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { GENTLE, collapse } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import { NudgeClient, POLL_MS, canNotify, iconFor, notify, type Nudge } from "../../lib/nudges";

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
  const { handshake, session } = useEngine();
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

    void poll();
    const timer = window.setInterval(() => void poll(), POLL_MS);
    return () => window.clearInterval(timer);
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
            <button
              type="button"
              onClick={() => {
                go(nudge.route);
                dismiss(nudge.key);
              }}
              className="text-accent hover:bg-accent/10 text-meta rounded-full px-2.5 py-1 font-medium"
            >
              {t("nudge.view")}
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

/** Whether to offer turning OS notifications on. Exported so Settings can show the same state. */
export function notificationsOn(): boolean {
  return canNotify();
}
