import { useEffect, useMemo, useState, type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { OnboardingClient, blocker, type Status } from "../../lib/onboarding";
import { Tour } from "./Tour";
import { seen } from "../../lib/tour";
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
/** Whether two answers from `/onboarding` say the same thing. */
function same(a: Status | null, b: Status): boolean {
  return a !== null && JSON.stringify(a) === JSON.stringify(b);
}

export function FirstRun({
  children,
  /**
   * Hold the tour back while something else owns the corner it lives in.
   *
   * The assistant panel opens down the right-hand side and its composer sits at the bottom of it —
   * exactly where the tour card is pinned. Somebody who opened the assistant during their first
   * session saw a panel inviting them to ask a question with the box to type it in covered up.
   */
  pauseTour = false,
}: {
  children: ReactNode;
  pauseTour?: boolean;
}) {
  const { handshake } = useEngine();
  const client = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const t = useT();
  const [status, setStatus] = useState<Status | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [tour, setTour] = useState(false);

  // Asked again, not once.
  //
  // This ran on mount and never again, so the banner that says a model is missing stayed up after
  // the model was installed — until the app was quit and reopened. A user installed Whisper, was
  // told to install a recogniser, and reasonably concluded the install had not worked.
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const again = () => setAttempt((n) => n + 1);
    window.addEventListener("focus", again);
    // While something is still wrong, keep checking: an install finishing is the event this is
    // waiting for, and it happens on another screen.
    const timer = window.setInterval(again, 5000);
    return () => {
      window.removeEventListener("focus", again);
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    client
      .status()
      .then((next) => {
        if (cancelled) return;
        // Only when it actually changed. The poll below runs every five seconds and the answer is
        // almost always identical; setting a fresh object anyway re-rendered the whole application
        // on a timer. Nothing looked wrong — but every CSS transition in the tree restarted five
        // times a minute, which is enough to make a button never settle: Playwright refused to
        // click one for thirty seconds, and a person on a slow machine pays for the same renders.
        setStatus((current) => (same(current, next) ? current : next));
        // The tour follows setup rather than running beside it: four cards over a download bar is
        // two things asking for attention at once.
        //
        // Only on the first answer. The status is polled now, and without this the tour would come
        // back five seconds after being closed, for as long as nobody had marked it seen.
        if (attempt === 0 && !next.should_prompt && !seen()) setTour(true);
      })
      .catch(() => {
        // No daemon yet. The status bar already says so; a setup screen on top of that would be a
        // second, less useful way of saying the same thing.
      });
    return () => {
      cancelled = true;
    };
  }, [client, attempt]);

  if (status?.should_prompt && !dismissed) {
    return (
      <Setup
        onDone={() => {
          setDismissed(true);
          // And ask the daemon again straight away. The answer on hand was fetched before the
          // install this screen just ran, so leaving setup dropped the user onto the home screen
          // under a banner saying a recogniser was missing — the very thing they had watched
          // download — until the five-second poll came round and took it away.
          setAttempt((n) => n + 1);
          if (!seen()) setTour(true);
        }}
      />
    );
  }

  // A broken install is a banner, not a takeover: the notes still open, the search still works, and
  // the one thing that does not work says so instead of hiding everything that does.
  const stuck = status ? blocker(status) : null;
  // Which half is missing, so the banner names it. `missing` is data the daemon already computes —
  // a vault with a recogniser and no voice detector was being told to install a recogniser, which
  // is both wrong and unfixable by doing what it says.
  const lacks = stuck?.missing ?? [];
  const part =
    stuck?.step === "models" && !lacks.includes("asr") && lacks.includes("vad")
      ? "vad"
      : (stuck?.step ?? "models");

  // A column rather than a fragment. The banner and the screen are siblings inside the scrolling
  // pane, and the screens size themselves to the full height of it — so a fragment made every screen
  // in the app exactly one banner taller than its container, which is a scrollbar on every screen
  // that has nothing to scroll to. The banner takes its own height, the screen takes the rest.
  return (
    <div className="flex h-full min-h-0 flex-col">
      {stuck && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta shrink-0 border-b px-4 py-2">
          <b className="font-medium">{t(`setup.step_${part}`)}</b> — {t(`setup.why_${part}`)}{" "}
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
      <div className="min-h-0 flex-1">{children}</div>
      {tour && !pauseTour && <Tour onClose={() => setTour(false)} />}
    </div>
  );
}
