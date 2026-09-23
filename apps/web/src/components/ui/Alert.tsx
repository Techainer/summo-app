import type { ReactNode } from "react";

import { cn } from "../../lib/cn";

/**
 * A line the screen has to say, in the colour of what it is saying.
 *
 * This existed already — seven times, hand-copied, in six files. The same string
 * (`border-{tone}/30 bg-{tone}-soft text-{tone} text-meta rounded-lg border px-3 py-2`) appeared in
 * the tasks screen, the page screen, the models screen twice over, the chat screen and the agents
 * screen, with only the tone token swapped. Every primitive in this folder exists because something
 * was copied one time too many; this is that, for the box that carries a warning.
 *
 * Three things the copies were each getting slightly wrong or not doing at all.
 *
 * **It is announced.** A failure a sighted reader sees appear was silent to a screen reader: not one
 * of the copies carried a role. A `danger` or `rec` alert interrupts (`role="alert"`), everything
 * else waits its turn (`role="status"`). The difference matters — interrupting somebody for "model
 * installed" is how people turn announcements off.
 *
 * **It uses the radius the system defines.** The copies said `rounded-lg`, which is Tailwind's own
 * 8px and not a token at all. The theme's radius for a thing you operate is `--radius-control`.
 *
 * **It can hold an action.** Several of the call sites needed a button beside the text and built the
 * row themselves, which is why the spacing between them differed by file.
 */
export type Tone = "accent" | "danger" | "blocked" | "rec";

const TONES: Record<Tone, string> = {
  accent: "border-accent/30 bg-accent-soft text-accent",
  danger: "border-danger/30 bg-danger-soft text-danger",
  blocked: "border-blocked/30 bg-blocked-soft text-blocked",
  rec: "border-rec/30 bg-rec-soft text-rec",
};

export function Alert({
  tone = "accent",
  icon,
  actions,
  className,
  children,
}: {
  tone?: Tone;
  /** Rendered before the text, and hidden from assistive technology — the tone is in the words. */
  icon?: ReactNode;
  /** A control that answers whatever this says, kept on the same row until there is no room. */
  actions?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      // Interrupting for good news is how people learn to turn announcements off, so only the two
      // tones that mean "something is wrong" take the reader away from what they were doing.
      role={tone === "danger" || tone === "rec" ? "alert" : "status"}
      className={cn(
        "text-meta rounded-control flex flex-wrap items-center gap-x-2 gap-y-1.5 border px-3 py-2",
        TONES[tone],
        className,
      )}
    >
      {icon && (
        <span aria-hidden="true" className="shrink-0">
          {icon}
        </span>
      )}
      <span className="min-w-0 flex-1">{children}</span>
      {actions}
    </div>
  );
}
