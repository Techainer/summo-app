import { useEffect, useMemo, useState, type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { OnboardingClient, blocker, type Status } from "../../lib/onboarding";
import { Tour, seen } from "./Tour";
import { Setup } from "./Setup";

/**
 * Decides what a user sees on launch: setup, the tour, or the app.
 *
 * The daemon decides whether setup is needed, not this component, and it recomputes the answer from
 * the machine every time. That is what makes the flow resumable — an install broken by a deleted
 * model directory prompts again, and a vault restored onto a new laptop does not get a welcome
 * screen it does not need.
 *
 * While the daemon has not answered, the app renders normally. Blocking the whole interface on a
 * loopback request would mean a blank window every launch for the sake of a screen almost nobody
 * needs twice.
 */
export function FirstRun({ children }: { children: ReactNode }) {
  const { handshake } = useEngine();
  const client = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const t = useT();
  const [status, setStatus] = useState<Status | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [tour, setTour] = useState(false);

  useEffect(() => {
    let cancelled = false;
    client
      .status()
      .then((next) => {
        if (cancelled) return;
        setStatus(next);
        // The tour follows setup rather than running beside it: four cards over a download bar is
        // two things asking for attention at once.
        if (!next.should_prompt && !seen()) setTour(true);
      })
      .catch(() => {
        // No daemon yet. The status bar already says so; a setup screen on top of that would be a
        // second, less useful way of saying the same thing.
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  if (status?.should_prompt && !dismissed) {
    return (
      <Setup
        onDone={() => {
          setDismissed(true);
          if (!seen()) setTour(true);
        }}
      />
    );
  }

  // A broken install is a banner, not a takeover: the notes still open, the search still works, and
  // the one thing that does not work says so instead of hiding everything that does.
  const stuck = status ? blocker(status) : null;

  return (
    <>
      {stuck && (
        <p className="border-b border-blocked/30 bg-blocked-soft px-4 py-2 text-[13px] text-blocked">
          <b className="font-medium">{t(`setup.step_${stuck.step}`)}</b> — {t(`setup.why_${stuck.step}`)}{" "}
          <button
            type="button"
            onClick={() => {
              setDismissed(false);
              setStatus((current) => (current ? { ...current, should_prompt: true } : current));
            }}
            className="font-medium underline"
          >
            {t("setup.fix")}
          </button>
        </p>
      )}
      {children}
      {tour && <Tour onClose={() => setTour(false)} />}
    </>
  );
}
