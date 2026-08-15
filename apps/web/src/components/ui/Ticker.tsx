import { useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef } from "react";

import { useI18n } from "../../i18n/context";

/**
 * A number that arrives by counting rather than by appearing.
 *
 * The home screen and the analytics cards are mostly large numerals, and a numeral that is simply
 * *there* on first paint reads as a placeholder — the same three digits sat in the same box whether
 * the report had loaded or not. Counting up says the figure was measured, and it says it in the
 * half second the eye is already travelling across the row.
 *
 * ## Why this is not a general-purpose animation
 *
 * It only runs when the value is a plain integer, and it only runs on the way *in*. Durations here
 * are strings like "1 giờ 12 phút" and rolling those through intermediate states would spell out
 * nonsense on the way; a number that changes because the user filtered the screen should snap,
 * because they are comparing it against what was there a moment ago.
 *
 * ## Accessibility
 *
 * The text content is written directly to the DOM node, so a screen reader that reaches the element
 * mid-count would read an intermediate figure. It is `aria-hidden` with the true value beside it in
 * a visually-hidden span, which is also what makes the count invisible to the screenshot audit's
 * text extraction. Under `prefers-reduced-motion` nothing counts: the final value is written on the
 * first frame.
 */
export function Ticker({ value, className }: { value: string; className?: string }) {
  const node = useRef<HTMLSpanElement>(null);
  const counted = useRef(false);
  const still = useReducedMotion();
  const { locale } = useI18n();

  // Localised digits and separators, so a count that ends at "1.234" does not pass through "1234".
  const format = useMemo(() => new Intl.NumberFormat(locale), [locale]);

  const target = countable(value);

  useEffect(() => {
    const element = node.current;
    if (!element) return;
    // Counted once, then never again for the life of this element.
    //
    // The analytics screen switches range — today, seven days, thirty — and the figure it holds
    // changes under a person who is comparing it against the one that was there a second ago.
    // Rolling that from zero every time turns a comparison into a wait, and it is the animation
    // equivalent of a page reload. Arrival is the only moment counting says anything true.
    if (target === null || still || counted.current) {
      element.textContent = value;
      return;
    }
    // Counted by hand rather than by the animation library.
    //
    // `animate()` from `motion` is the same engine that drives every animated element — keyframes,
    // springs, interpolation between colours and transforms — and importing it put all of that in
    // the chunk the browser parses before the first paint, for a number going up. Every animated
    // *element* is loaded on the side (see `main.tsx`); this was the one import that dragged the
    // engine back into the entry chunk, because the home screen is made of these.
    //
    // What is lost: nothing this uses. One value, one duration, one curve, no interruption — the
    // count runs once per element and stops.
    const span = Math.min(900, 250 + target * 20);
    let frame = 0;
    let started: number | null = null;
    const step = (now: number) => {
      started ??= now;
      const t = Math.min(1, (now - started) / span);
      // Expo out, which is what `[0.16, 1, 0.3, 1]` was: nearly all of the distance in the first
      // third, then a long settle. A count that eases in reads as hesitation.
      const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
      element.textContent = format.format(Math.round(target * eased));
      if (t < 1) {
        frame = requestAnimationFrame(step);
        return;
      }
      // On completion rather than on start, so an animation that was torn down before it finished
      // — React's development double-invoke does exactly this — is allowed to run again.
      counted.current = true;
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target, value, still, format]);

  return (
    <>
      <span ref={node} aria-hidden="true" className={className}>
        {target === null || still ? value : "0"}
      </span>
      <span className="sr-only">{value}</span>
    </>
  );
}

/**
 * The number to count to, or `null` for anything that is not a bare integer.
 *
 * Deliberately strict. "12" counts; "1 giờ 12 phút", "1.2 GB" and "—" do not, and each of those is
 * a real value one of these cards holds.
 */
function countable(value: string): number | null {
  if (!/^\d{1,6}$/.test(value)) return null;
  const parsed = Number(value);
  // Nothing to watch, and a zero that counts to zero is a flicker.
  return parsed > 0 ? parsed : null;
}
