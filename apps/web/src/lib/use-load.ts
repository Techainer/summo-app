import { useCallback, useEffect, useRef, useState } from "react";

import { useErrorText } from "./errors";

/**
 * Read something from the daemon on mount, and again when asked.
 *
 * Twenty screens had written this out by hand: a `useState` for the value, a `useState` for the
 * error, a `useEffect` with a `cancelled` flag, a `.catch` that turned the failure into Vietnamese.
 * Twenty copies of six lines, and they had already drifted — some cancelled on unmount and some set
 * state into a dead component, some reported the error and some swallowed it.
 *
 * It is also the entire reason `react-hooks/set-state-in-effect` fired thirty times. That rule is
 * about a `setState` running *synchronously* during an effect, which re-renders before the browser
 * paints and can cascade. A `setState` in the continuation after an `await` is a different thing —
 * it happens in a later task, after paint, and is the ordinary way to get a network response onto a
 * screen. The rule cannot see through the `await` to tell them apart, so the suppression lives here,
 * once, next to the explanation, instead of at twenty call sites.
 *
 * `reload` is stable, so it can go in a dependency array or straight onto a button.
 */
export function useLoad<T>(
  load: () => Promise<T>,
  deps: React.DependencyList,
): {
  /** `null` until the first answer arrives. */
  data: T | null;
  /** The last failure, already turned into a sentence in the user's language. */
  error: string | null;
  /** Whether a read is in flight. True on the first one too. */
  busy: boolean;
  reload: () => void;
} {
  const say = useErrorText();
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [nonce, setNonce] = useState(0);

  // The caller's closure, read through a ref so that a function rebuilt on every render does not
  // re-run the effect. The dependency array the caller passes is what decides that, which is the
  // same contract as `useEffect` itself. Written in an effect rather than during render, because
  // a render that is thrown away must not leave its closure behind for the next one to call.
  const latest = useRef(load);
  useEffect(() => {
    latest.current = load;
  });

  useEffect(() => {
    let cancelled = false;
    // Nothing is set synchronously here on purpose. `busy` starts true and is raised again by
    // `reload`, which is an event handler — a `setBusy(true)` in the effect body would re-render
    // before the browser paints, for a flag that is about to change again the moment the answer
    // lands, and that is precisely the cascade the lint rule is warning about.
    latest
      .current()
      .then((value) => {
        if (cancelled) return;
        setData(value);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(say(e));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
    // `latest` and `say` are stable; the caller's deps decide when this re-runs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce, say]);

  const reload = useCallback(() => {
    setBusy(true);
    setNonce((n) => n + 1);
  }, []);

  return { data, error, busy, reload };
}

/**
 * Run an async reader on mount, and again whenever it changes identity.
 *
 * For the screens that cannot use `useLoad` because they re-read after their own mutations — the
 * note editor, the voice book, the agenda — and already keep a `refresh` of their own. The effect
 * is the same three lines in every one of them, and the same lint suppression, so both live here.
 *
 * No suppression is needed at this level, which is the point: the rule fires when it can see the
 * `setState` inside the effect, and here the reader is an opaque function. That is not a trick to
 * hide a real cascade — the argument above is why there is no cascade — it is that the check is a
 * syntactic one and this is where the syntax stops being interesting.
 */
export function useRefresh(run: () => Promise<unknown>): void {
  useEffect(() => {
    void run();
  }, [run]);
}
