import type { InputHTMLAttributes, TextareaHTMLAttributes } from "react";

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
