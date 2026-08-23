import { useEffect, useRef, useState } from "react";

import type { Install } from "./onboarding";

/**
 * How fast a download is going, and how long it has left.
 *
 * A percentage is not enough for these files. SMALL100 is 611 MB and the registry's host has been
 * measured at around 200 KB/s from Vietnam, which is fifty minutes — and over fifty minutes a
 * percentage advances one point every thirty seconds. A user watched a spinner and a number that
 * did not visibly move, concluded it had hung, and pressed the button again. The download was
 * working perfectly the entire time.
 *
 * So the answer to "why is it just spinning" is not a nicer spinner: it is the two facts a spinner
 * cannot carry. `18.3 MB of 611 MB · 210 KB/s · about 47 minutes left` is a slow download. A
 * spinner is a hang.
 *
 * ## Why a window rather than an average
 *
 * Averaging from the start is wrong in the direction that matters: a download that stalls for two
 * minutes and resumes reports the rate it had before stalling, so the estimate keeps promising a
 * finish it has already missed. Averaging only the last few samples tracks what is happening now,
 * which is what somebody deciding whether to wait actually needs.
 */

/** How many one-second samples the rate is averaged over. */
const WINDOW = 8;

/**
 * The smallest span worth dividing by.
 *
 * Two samples a few milliseconds apart — a re-render, a duplicated poll — produce a rate in the
 * gigabytes and an estimate of zero, which flickers on screen and is never true.
 */
const MIN_SPAN_MS = 750;

export interface Progress {
  /** Bytes per second over the recent window, or `null` before there is enough to say. */
  rate: number | null;
  /** Seconds remaining at the current rate, or `null` when the total or the rate is unknown. */
  eta: number | null;
}

/**
 * Watch an install and report its speed.
 *
 * Samples come from re-renders, which happen as the poll in the calling component updates the job.
 * Nothing is timed here: a hook that ran its own interval would keep a component re-rendering after
 * the download finished, and there is already a poll doing exactly this cadence.
 */
export function useProgress(install: Install | null): Progress {
  const samples = useRef<{ at: number; done: number }[]>([]);
  const model = useRef<string | null>(null);
  const [progress, setProgress] = useState<Progress>({ rate: null, eta: null });

  const done = install?.done ?? null;
  const total = install?.total ?? null;
  const state = install?.state ?? null;
  const id = install?.model ?? null;

  useEffect(() => {
    // A different model, or a fresh run of the same one: the previous samples describe a download
    // that is not this one, and carrying them over would report the old file's speed for the first
    // few seconds of the new one.
    if (model.current !== id || state === "queued") {
      model.current = id;
      samples.current = [];
      setProgress({ rate: null, eta: null });
      return;
    }
    if (state !== "downloading" || done === null) return;

    const now = performance.now();
    const last = samples.current[samples.current.length - 1];
    // Byte counts that have not moved still get recorded — a stall is information, and dropping
    // the samples would freeze the reported rate at whatever it was before the stall.
    if (last && now - last.at < MIN_SPAN_MS) return;
    samples.current = [...samples.current, { at: now, done }].slice(-WINDOW);

    const first = samples.current[0];
    const latest = samples.current[samples.current.length - 1];
    if (!first || !latest || samples.current.length < 2) return;

    const span = latest.at - first.at;
    const moved = latest.done - first.done;
    if (span < MIN_SPAN_MS) return;

    // Negative is possible when a resumed download restarts a chunk. Reported as unknown rather
    // than as a negative speed, which would render as a time estimate in the past.
    const perSecond = moved <= 0 ? 0 : (moved / span) * 1000;
    setProgress({
      rate: perSecond,
      eta:
        total !== null && total > latest.done && perSecond > 0
          ? Math.round((total - latest.done) / perSecond)
          : null,
    });
  }, [id, state, done, total]);

  return progress;
}

/**
 * A transfer rate, in the units a download manager uses.
 *
 * Decimal rather than binary, matching `onboarding.size`'s neighbours on screen: two different
 * conventions for a megabyte in one sentence is worse than either convention.
 */
export function perSecond(bytes: number | null): string {
  if (bytes === null || bytes <= 0) return "";
  if (bytes < 1000) return `${Math.round(bytes)} B/s`;
  if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} KB/s`;
  return `${(bytes / 1_000_000).toFixed(1)} MB/s`;
}

/**
 * Seconds as a rough duration, rounded the way a person would say it.
 *
 * Deliberately coarse above a minute. A download with eleven minutes left does not have eleven
 * minutes and four seconds left, and a seconds field ticking down beside a fifty-minute estimate
 * claims a precision the estimate does not have.
 */
export function roughly(seconds: number | null): { unit: "sec" | "min" | "hour"; value: number } | null {
  if (seconds === null || seconds <= 0 || !Number.isFinite(seconds)) return null;
  // Five is the floor as well as the step. "About 1 second left" is a claim the estimate cannot
  // support, and it is followed by several more seconds of the same sentence.
  if (seconds < 90) return { unit: "sec", value: Math.max(5, Math.round(seconds / 5) * 5) };
  if (seconds < 5400) return { unit: "min", value: Math.max(1, Math.round(seconds / 60)) };
  return { unit: "hour", value: Math.round((seconds / 3600) * 10) / 10 };
}
