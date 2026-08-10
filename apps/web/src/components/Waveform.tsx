import { useEffect, useRef } from "react";

/**
 * A live level meter.
 *
 * Its job is not decoration: it is the fastest way for someone to tell that the microphone is
 * actually picking them up. "The app is recording but hears nothing" and "the app is working" look
 * identical without it, and the first is the most common way a first attempt fails.
 */
export function Waveform({ level, active }: { level: number; active: boolean }) {
  const history = useRef<number[]>(Array.from({ length: 28 }, () => 0));
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
      {history.current.map((_, index) => (
        <i key={index} />
      ))}
    </div>
  );
}
