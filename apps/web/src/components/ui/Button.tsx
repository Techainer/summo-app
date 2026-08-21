import { Slot } from "@radix-ui/react-slot";
import type { ButtonHTMLAttributes, ReactNode } from "react";

import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:brightness-110 active:brightness-95",
  secondary: "bg-bg-soft text-fg border border-line hover:border-line-strong",
  ghost: "text-fg-dim hover:text-fg hover:bg-bg-soft",
  danger: "bg-rec text-white hover:brightness-110",
};

const SIZES: Record<Size, string> = {
  sm: "h-8 px-3 text-meta gap-1.5",
  md: "h-9 px-4 text-sm gap-2",
  lg: "h-11 px-5 text-body gap-2",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  /** Render as the single child element instead of a `<button>`, for links that look like buttons. */
  asChild?: boolean;
  /** Show a spinner and block interaction. The label stays, so the button does not change width. */
  busy?: boolean;
  icon?: ReactNode;
}

export function Button({
  variant = "secondary",
  size = "md",
  asChild = false,
  busy = false,
  icon,
  className,
  children,
  disabled,
  ...props
}: ButtonProps) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      className={cn(
        "inline-flex items-center justify-center rounded-full font-medium",
        // A label never wraps. `h-9` is a fixed height, so a two-line label does not make the
        // button taller — it overflows it, which is what "Xác nhận" was doing on the draft panel
        // in every screenshot the marketing site was built from. Vietnamese is the case that
        // catches this: its words are short but many, so a label that fits in English arrives
        // here as three tokens the flex container is happy to break.
        "whitespace-nowrap",
        "transition-[background,border-color,filter,opacity,transform] duration-150",
        "disabled:pointer-events-none disabled:opacity-[var(--disabled-opacity)]",
        // Pressed, not just hovered. A button that does not move under the finger is the single
        // most common reason an interface feels dead, and it costs one transform.
        "active:scale-[0.97]",
        // The ring sits off the edge rather than on it, so it stays visible on a control whose own
        // border is already the accent colour — which is what the focus ring is drawn in.
        "focus-visible:ring-accent focus-visible:ring-2 focus-visible:ring-offset-[var(--ring-offset)]",
        "focus-visible:ring-offset-bg focus-visible:outline-none",
        VARIANTS[variant],
        SIZES[size],
        className,
      )}
      disabled={disabled || busy}
      // Tells a screen reader the control is working rather than broken.
      aria-busy={busy || undefined}
      {...props}
    >
      {busy ? <Spinner /> : icon}
      {children}
    </Comp>
  );
}

export function Spinner({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-block size-3.5 rounded-full border-2 border-current border-r-transparent",
        "motion-safe:animate-spin",
        className,
      )}
      aria-hidden="true"
    />
  );
}
