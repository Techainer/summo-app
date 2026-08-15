import * as Dialog from "@radix-ui/react-dialog";
import { AnimatePresence, m } from "motion/react";
import type { ReactNode } from "react";

import { SNAPPY, SPRING } from "../../lib/motion";
import { cn } from "../../lib/cn";

/**
 * A panel that slides in from an edge.
 *
 * Used for the sidebar on narrow screens and for pickers on mobile, where the fifth reference
 * design puts language and speed choices in a bottom sheet rather than a dropdown — a dropdown
 * anchored near the bottom of a phone screen opens under the user's thumb.
 *
 * Built on Radix `Dialog` so focus trapping, `Escape`, scroll locking and `aria-modal` come for
 * free; getting those right by hand is where hand-rolled sheets usually fail.
 */
export function Sheet({
  open,
  onOpenChange,
  side = "left",
  title,
  description,
  children,
  className,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  side?: "left" | "right" | "bottom";
  /** Required: a modal surface with no accessible name is unusable with a screen reader. */
  title: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  const enter = side === "bottom" ? { y: "100%" } : { x: side === "left" ? "-100%" : "100%" };
  const settled = side === "bottom" ? { y: 0 } : { x: 0 };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <Dialog.Portal forceMount>
            <Dialog.Overlay asChild>
              <m.div
                className="fixed inset-0 z-40 bg-black/50"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={SNAPPY}
              />
            </Dialog.Overlay>
            <Dialog.Content asChild>
              <m.div
                className={cn(
                  "bg-bg-raised border-line fixed z-50 shadow-[var(--shadow-pop)]",
                  side === "left" && "inset-y-0 left-0 w-[290px] border-r",
                  side === "right" && "inset-y-0 right-0 w-[360px] border-l",
                  side === "bottom" &&
                    "inset-x-0 bottom-0 max-h-[80vh] rounded-t-[var(--radius-panel)] border-t",
                  className,
                )}
                initial={enter}
                animate={settled}
                exit={enter}
                transition={SPRING}
              >
                {side === "bottom" && (
                  <div className="flex justify-center pt-2.5" aria-hidden="true">
                    <span className="bg-line-strong h-1 w-9 rounded-full" />
                  </div>
                )}
                <Dialog.Title className="sr-only">{title}</Dialog.Title>
                {description && (
                  <Dialog.Description className="sr-only">{description}</Dialog.Description>
                )}
                {children}
              </m.div>
            </Dialog.Content>
          </Dialog.Portal>
        )}
      </AnimatePresence>
    </Dialog.Root>
  );
}
