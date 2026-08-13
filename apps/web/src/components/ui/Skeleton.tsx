import { cn } from "../../lib/cn";

/**
 * The shape of something that has not arrived yet.
 *
 * The screens said `Đang tải…` in grey, centred, which tells a person nothing about what is coming
 * and makes the layout jump when it does. A block the size of the thing being fetched keeps the
 * page still and says "three cards, in a moment" without a word.
 *
 * A sweep of light rather than a pulse. A pulsing block reads as a warning; a sweep reads as
 * loading, which is the convention every product the references came from uses. Both stop under
 * `prefers-reduced-motion` — the rule for that lives in `theme.css` and applies to everything.
 */
export function Skeleton({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "bg-bg-soft relative block overflow-hidden rounded-[var(--radius-card)]",
        // The sweep is a child rather than a background-position animation: a gradient sliding
        // across a `background-size: 200%` costs a repaint of the whole box each frame, and a
        // transformed child is composited.
        "after:absolute after:inset-0 after:-translate-x-full",
        "after:animate-[shimmer_1.6s_ease-in-out_infinite]",
        "after:bg-[linear-gradient(90deg,transparent,var(--color-bg-elevated),transparent)]",
        className,
      )}
    />
  );
}
