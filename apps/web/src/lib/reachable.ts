import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Whether the daemon is still there.
 *
 * The app is a page served by a process on the same machine, and that process can go away while the
 * page stays open: it crashes, it is quit from the tray, the machine sleeps and the port is gone
 * when it wakes. Nothing in the interface noticed. Driven with the daemon killed under it, the app
 * kept its whole layout, kept the meetings it had already fetched on screen, and let a note be
 * typed into for four seconds — every request failing with `ERR_CONNECTION_REFUSED` in a console no
 * user reads — without a word about it. What is typed in that state is gone on reload.
 *
 * Recording already has this: the WebSocket reconnects and the status bar says so. But the socket
 * only exists while a recording does, so every other minute of the app's life was unwatched.
 *
 * `/health` is the check because it is the one route with no token: a daemon answering it is a
 * daemon that is up, and a 401 from a stale token is a different problem with a different message.
 */

/** How often to look while it is answering. Local, so the request costs nothing worth counting. */
const WELL_MS = 5000;

/** How often to look while it is not. Faster, because this is now a person waiting to be let back in. */
const ILL_MS = 1500;

/**
 * How many misses before saying so.
 *
 * One is not enough. A daemon restarting after an update, or a machine coming out of sleep, drops a
 * single request and comes straight back; a bar that flashes on every hiccup is a bar people learn
 * to ignore. Two misses at `ILL_MS` apart is under four seconds — still faster than a person
 * finishes the sentence they are typing.
 */
const MISSES = 2;

export interface Watch {
  /** Consecutive failed checks. */
  misses: number;
  /** What to tell the user. Stays `true` until `MISSES` in a row have failed. */
  reachable: boolean;
}

export const FRESH: Watch = { misses: 0, reachable: true };

/** Fold one check's result into the watch. */
export function saw(watch: Watch, ok: boolean): Watch {
  const misses = ok ? 0 : watch.misses + 1;
  return { misses, reachable: misses < MISSES };
}

/** How long to wait before the next check. */
export function delay(watch: Watch): number {
  // On the *first* miss too: something is wrong and the next answer decides what to say, so it is
  // worth asking sooner rather than sitting out a full healthy interval before deciding.
  return watch.misses === 0 ? WELL_MS : ILL_MS;
}

/**
 * Poll the daemon, and say when it stops answering.
 *
 * Paused while the tab is hidden — a background tab polling a dead port helps nobody — and checked
 * immediately when it comes back, which is also the moment a user would notice.
 */
export function useReachable(port: number): { reachable: boolean; check: () => void } {
  const [watch, setWatch] = useState<Watch>(FRESH);
  /**
   * The watch as the loop last left it, so a restart carries on from what is known.
   *
   * Starting a new loop from `FRESH` would hide the bar the instant somebody pressed retry and put
   * it back a second later, which reads as the app losing its nerve rather than checking.
   */
  const latest = useRef<Watch>(FRESH);
  const [asked, setAsked] = useState(0);

  /** Ask again now: the retry button, and the tab coming back to the front. */
  const check = useCallback(() => setAsked((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    const tick = async () => {
      let ok: boolean;
      try {
        const response = await fetch(`http://127.0.0.1:${port}/health`, { cache: "no-store" });
        ok = response.ok;
      } catch {
        // A refused connection, a reset, a name that no longer resolves — one fact: nothing
        // answered.
        ok = false;
      }
      if (cancelled) return;
      const next = saw(latest.current, ok);
      latest.current = next;
      setWatch(next);
      timer = window.setTimeout(() => void tick(), delay(next));
    };

    const stop = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = null;
    };

    const visibility = () => {
      stop();
      if (document.visibilityState === "visible") void tick();
    };

    void tick();
    document.addEventListener("visibilitychange", visibility);
    return () => {
      cancelled = true;
      stop();
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [port, asked]);

  return { reachable: watch.reachable, check };
}
