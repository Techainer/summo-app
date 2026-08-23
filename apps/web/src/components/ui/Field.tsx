import type { InputHTMLAttributes, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "../../lib/cn";

/**
 * One input, everywhere.
 *
 * There were twenty-four `<input>` and `<textarea>` elements in this app and no two agreed. Some
 * were `rounded-full border px-2.5 py-1 text-meta`, some `rounded-xl border px-3 py-2 text-sm`, one
 * was a bare `rounded-md`. Height, radius, focus colour and the size of the text inside all varied,
 * so a form was a row of controls that had never met. That is the kind of thing nobody can name
 * when they look at a screen and say it feels unfinished, and it is most of why they say it.
 *
 * Fixed here: 36px tall at `md`, the card radius, the page's own line colour, and the focus ring
 * from `Button` so a tabbed-to field and a tabbed-to button look like the same interface. Padding
 * is logical (`ps`/`pe`) rather than left/right, because Summo ships in four languages and adding a
 * fifth that reads right to left should not be a search through the styles.
 */
const BASE = [
  "w-full min-w-0 rounded-[var(--radius-card)] border transition-colors",
  "border-line bg-bg-soft text-fg placeholder:text-fg-faint",
  "hover:border-line-strong",
  "focus-visible:border-accent focus-visible:ring-accent/25 focus-visible:ring-2 focus:outline-none",
  "disabled:pointer-events-none disabled:opacity-[var(--disabled-opacity)]",
].join(" ");

const SIZES = {
  sm: "h-8 px-2.5 text-meta",
  md: "h-9 px-3 text-sm",
} as const;

export function Input({
  size = "md",
  className,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, "size"> & { size?: keyof typeof SIZES }) {
  return <input className={cn(BASE, SIZES[size], className)} {...props} />;
}

export function TextArea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={cn(BASE, "resize-y px-3 py-2 text-sm", className)} {...props} />;
}

/**
 * One dropdown, everywhere — the control the pass above forgot.
 *
 * `Input` and `TextArea` were unified and the sixteen `<select>` elements were not, so every form in
 * the app pairs a 36px input against a dropdown the operating system sized itself. That is what
 * "the dropdowns are misaligned" means: on macOS a native select is 21px tall with its own rounding
 * and its own inset shadow, so a field and the dropdown beside it sit on different baselines, in
 * different shapes, with different focus rings. Two of the call sites had also drifted to their own
 * spellings of the same border classes.
 *
 * `appearance-none` is what makes that fixable — it costs the native arrow, so one is drawn back.
 * The wrapper exists only to position it; the arrow is `pointer-events-none` so the whole control
 * still opens on a click, and the end padding leaves room so a long option never runs underneath.
 *
 * Padding is logical and the chevron is placed with `end-*`, for the same reason as the base: a
 * right-to-left locale should move the arrow without anybody editing this file.
 *
 * Unlike `Input`, `className` lands on the *wrapper* rather than the control. Every caller was
 * using it for layout — `flex-1`, `ms-auto` — and layout applied to a control inside a positioned
 * wrapper does nothing at all, which would have made this a silent regression at each call site.
 * Height and text size are `size` instead, which is the part that has to stay on the select.
 */
export function Select({
  size = "md",
  className,
  ...props
}: Omit<SelectHTMLAttributes<HTMLSelectElement>, "size"> & { size?: keyof typeof SIZES }) {
  return (
    <span className={cn("relative block min-w-0", className)}>
      <select className={cn(BASE, SIZES[size], "cursor-pointer appearance-none pe-8")} {...props} />
      <ChevronDown
        aria-hidden="true"
        className="text-fg-faint pointer-events-none absolute end-2.5 top-1/2 size-4 -translate-y-1/2"
      />
    </span>
  );
}

/**
 * A label above a control, with room for a note under it.
 *
 * `<label>` wrapping the control rather than `htmlFor`, so a caller cannot forget the id and leave
 * a label pointing at nothing — which is a label that reads aloud as a sentence with no control
 * attached to it.
 */
export function Labelled({
  label,
  hint,
  children,
  className,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("flex flex-col gap-1.5", className)}>
      <span className="text-meta font-medium">{label}</span>
      {children}
      {hint && <span className="text-fg-faint text-micro">{hint}</span>}
    </label>
  );
}
