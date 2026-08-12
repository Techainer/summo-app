import type { ReactNode } from "react";

import { cn } from "../../lib/cn";

/**
 * A real checkbox, hidden and painted through its own label.
 *
 * The browser's default is a blue square drawn by the operating system. Next to a dark,
 * green-accented interface it reads as something the page did not mean to include — and it was on
 * two screens with two different bits of markup around it, so fixing one left the other.
 *
 * `sr-only` on the input rather than `appearance-none`: that keeps every part of the native
 * behaviour — space to toggle, tab order, the accessibility tree, the label association — and
 * changes only the paint. Rebuilding the focus ring and the hit target by hand is how a custom
 * control ends up unusable by keyboard.
 */
export function Checkbox({
  checked,
  onChange,
  disabled,
  children,
  className,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label
      className={cn(
        "group text-fg-dim text-meta flex cursor-pointer items-center gap-2.5 transition-colors",
        "has-[:checked]:text-fg has-[:disabled]:cursor-default has-[:disabled]:opacity-60",
        className,
      )}
    >
      <input
        type="checkbox"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span
        aria-hidden="true"
        className={cn(
          "border-line-strong grid size-4 shrink-0 place-items-center rounded-[5px] border",
          "text-accent-fg text-[10px] transition-colors",
          "peer-checked:border-accent peer-checked:bg-accent",
          "peer-focus-visible:ring-accent peer-focus-visible:ring-2 peer-focus-visible:ring-offset-1",
          "peer-focus-visible:ring-offset-bg",
        )}
      >
        {/* A descendant of the box, not a sibling of the input, so `peer-checked:` cannot reach it
            — that variant compiles to a sibling combinator. `group-has-[:checked]:` can. */}
        <span className="opacity-0 transition-opacity group-has-[:checked]:opacity-100">✓</span>
      </span>
      {children}
    </label>
  );
}
