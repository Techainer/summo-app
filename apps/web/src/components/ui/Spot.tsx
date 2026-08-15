import type { LucideIcon } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";

import { cn } from "../../lib/cn";

/**
 * The drawing on an empty screen.
 *
 * ## Why this is drawn here rather than downloaded
 *
 * The obvious move is a free illustration or Lottie pack. Two things rule it out. The good free
 * animation libraries are licensed per asset on a website rather than shipped as a package with a
 * licence file, and this repository is open source — art with an unrecorded provenance is a problem
 * somebody inherits, not one they find. And a Lottie player is 60–100 kB gzipped before a single
 * frame, for decoration, in an app whose whole first load is budgeted at 300 kB.
 *
 * The larger reason is that a downloaded illustration is drawn in somebody else's palette. It looks
 * imported in light mode and wrong in dark mode, and there is no version of it that answers to
 * `--color-accent`. This is four circles and a glyph; it costs about two kilobytes, it is the
 * theme's own colours, and it is the same drawing at every size.
 *
 * ## What it does
 *
 * A tinted blob that drifts, a ring that breathes, two motes on slow orbits, and the screen's icon
 * in the middle. Slow on purpose — 6 to 11 seconds a cycle, no two the same, so nothing pulses in
 * time with anything else. An empty screen should feel unhurried and alive, not like a loading
 * state, and anything faster than this reads as "wait".
 *
 * Under `prefers-reduced-motion` it is a still picture. The composition carries it; the movement is
 * the part nobody needs.
 */
export function Spot({
  icon: Icon,
  className,
  size = 96,
}: {
  icon: LucideIcon;
  className?: string;
  /** Edge of the square the whole scene is drawn in. */
  size?: number;
}) {
  const still = useReducedMotion();

  return (
    <div
      aria-hidden="true"
      // `data-ink` says "this draws something" to `e2e/density.mjs`, which decides whether a screen
      // is a composition or a hole. Its walker looks for text and for a handful of tag names, and a
      // drawing made of spans and gradients is neither — so an illustrated empty state measured as
      // empty from the top of its *heading*, and the screen was reported lopsided the moment the
      // picture grew. Declaring it is cheaper and more honest than teaching a test to recognise art.
      data-ink=""
      className={cn("relative grid shrink-0 place-items-center", className)}
      style={{ width: size, height: size }}
    >
      {/* The blob. Two overlapping radial tints rather than one, so the colour has somewhere to
          move to — a single gradient that changes opacity reads as a light being switched. */}
      <motion.span
        className="from-accent/35 via-accent/10 absolute inset-0 rounded-[42%] bg-linear-to-br to-transparent"
        animate={still ? undefined : { rotate: 360, borderRadius: ["42%", "54%", "42%"] }}
        transition={{ duration: 22, repeat: Infinity, ease: "linear" }}
      />
      <motion.span
        className="from-ai/30 absolute inset-[8%] rounded-[48%] bg-linear-to-tl to-transparent"
        animate={still ? undefined : { rotate: -360 }}
        transition={{ duration: 17, repeat: Infinity, ease: "linear" }}
      />

      {/* The ring, breathing. It is what stops the blob from looking like a stain. */}
      <motion.span
        className="ring-line absolute inset-[18%] rounded-full ring-1"
        animate={still ? undefined : { scale: [1, 1.06, 1], opacity: [0.7, 1, 0.7] }}
        transition={{ duration: 6.5, repeat: Infinity, ease: "easeInOut" }}
      />

      {/* Two motes, on orbits of different periods so they never line up twice the same way. */}
      {[
        { at: 0, seconds: 9, place: "top-[3%]", size: "size-2", tint: "bg-accent/70" },
        { at: 1, seconds: 11.5, place: "bottom-[8%]", size: "size-1.5", tint: "bg-ai/70" },
      ].map((mote) => (
        <motion.span
          key={mote.at}
          className="absolute inset-0"
          animate={still ? undefined : { rotate: 360 }}
          transition={{ duration: mote.seconds, repeat: Infinity, ease: "linear" }}
        >
          {/* Placed on the rim of the rotating layer, so the radius comes from the layout rather
              than from a number that has to be recomputed for every size this is drawn at. */}
          <span
            className={cn(
              "absolute left-1/2 -translate-x-1/2 rounded-full",
              mote.place,
              mote.size,
              mote.tint,
            )}
          />
        </motion.span>
      ))}

      <span className="bg-bg-raised ring-line relative grid size-[46%] place-items-center rounded-full shadow-[var(--shadow-sm)] ring-1">
        <Icon className="text-fg-faint size-[46%] stroke-[1.5]" />
      </span>
    </div>
  );
}
