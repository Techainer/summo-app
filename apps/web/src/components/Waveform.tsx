import { useEffect, useRef } from "react";

/**
 * A live level meter.
 *
 * Its job is not decoration: it is the fastest way for someone to tell that the microphone is
 * actually picking them up. "The app is recording but hears nothing" and "the app is working" look
 * identical without it, and the first is the most common way a first attempt fails.
 */
/** Bars in the meter. A constant, because the number of them is not something that varies. */
const BARS = 28;

export function Waveform({ level, active }: { level: number; active: boolean }) {
  const history = useRef<number[]>(Array.from({ length: BARS }, () => 0));
  const container = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!active) {
      history.current = history.current.map(() => 0);
    } else {
      // A perceptual curve: speech sits low in linear amplitude, and a linear bar barely moves.
      history.current = [...history.current.slice(1), Math.min(1, Math.sqrt(level) * 1.8)];
    }

    const element = container.current;
    if (!element) return;
    const bars = element.children;
    for (let i = 0; i < bars.length; i += 1) {
      const bar = bars[i] as HTMLElement | undefined;
      const value = history.current[i] ?? 0;
      if (bar) bar.style.transform = `scaleY(${Math.max(0.06, value)})`;
    }
  }, [level, active]);

  return (
    <div
      ref={container}
      className={active ? "waveform active" : "waveform"}
      aria-hidden
      data-testid="waveform"
    >
      {/* From the constant, not from the ref. Reading a ref during render is what React forbids,
          and here it was reading it only to count to 28 — the levels themselves never render, they
          are written straight onto the bars' transforms by the effect above, which is the whole
          reason this meter does not re-render sixty times a second. */}
      {Array.from({ length: BARS }, (_, index) => (
        <i key={index} />
      ))}
    </div>
  );
}
