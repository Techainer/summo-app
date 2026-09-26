import { useCallback, useEffect, useMemo, useState } from "react";

import type { Handshake } from "./engine";
import { OnboardingClient, POLL_MS, isFinished, type Install } from "./onboarding";

/**
 * Start a download and watch it, from anywhere that offers one.
 *
 * The models screen has drawn a real bar — how far, how fast, how long left — since `Progress`
 * existed. Every *other* place that offers a download drew nothing at all: the recognition panel's
 * "install and use this one", the voice-over panel's suggestion, the first-run checklist. They
 * posted to `/installs` and returned, so a 600 MB model downloaded in silence behind a button that
 * had stopped looking busy.
 *
 * That is worse than the spinner `Progress` was written to replace, because at least a spinner is
 * present. A user pressed the button, nothing visible happened, and the only honest conclusion
 * available to them was that the button was broken.
 *
 * So the polling lives here rather than being written a fourth time. The daemon keys `/installs`
 * by model id — asking twice for the same model is somebody pressing twice, and the right answer
 * is the job already running — which is why this needs no job id of its own.
 */
export function useInstall(handshake: Handshake) {
  const client = useMemo(() => new OnboardingClient(handshake), [handshake]);
  const [watching, setWatching] = useState<string | null>(null);
  const [job, setJob] = useState<Install | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!watching) return undefined;
    let cancelled = false;

    const ask = () => {
      client
        .installs()
        .then((all) => {
          if (cancelled) return;
          const mine = all.find((one) => one.model === watching);
          if (!mine) return;
          setJob(mine);
          // Stop polling, keep the job: a finished download still has something to say — `failed`
          // carries the reason, and a caller may want to read `done` back.
          if (isFinished(mine)) setWatching(null);
        })
        .catch(() => undefined);
    };

    ask();
    const timer = window.setInterval(ask, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [watching, client]);

  const start = useCallback(
    async (id: string) => {
      setError(null);
      setJob(null);
      try {
        const started = await client.install(id);
        setJob(started);
        setWatching(id);
        return true;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return false;
      }
    },
    [client],
  );

  return {
    /** The job, while there is one to show. `null` before the first press. */
    job,
    /** Whether bytes are still moving, for a button that should stay busy. */
    running: job !== null && !isFinished(job),
    /** Why the *request* failed. A download that starts and then fails reports through `job`. */
    error,
    start,
  };
}
