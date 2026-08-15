import { m } from "motion/react";

import { cn } from "../../lib/cn";
import { SNAPPY } from "../../lib/motion";
import type { NavItem } from "./Sidebar";

/**
 * The four places worth a thumb, and the button that records.
 *
 * Below the breakpoint the only way to change screen was the hamburger: tap it, wait for a sheet,
 * read eleven rows, tap again. Three actions and a full-screen overlay to move between two places
 * somebody moves between constantly — and on a phone the hamburger sits in the top-left, which is
 * the hardest corner for a right thumb to reach.
 *
 * So the destinations that get used move to the bottom, where the hand already is. The sheet stays
 * for everything else, which is the right place for the machinery: settings and model management
 * are not one-handed tasks.
 *
 * Four, not five or six. A bar of six on a 390 px screen gives each target 65 px, which is under
 * the 44 px minimum once the padding is honest about itself, and labels start truncating into
 * unreadable stems.
 */
export function BottomBar({
  items,
  active,
  onNavigate,
  recording,
  onRecord,
  recordLabel,
}: {
  /** Four, in the order they appear. Anything longer is silently trimmed rather than squeezed. */
  items: NavItem[];
  active: string;
  onNavigate: (key: string) => void;
  recording: boolean;
  onRecord: () => void;
  recordLabel: string;
}) {
  const shown = items.slice(0, 4);
  const left = shown.slice(0, 2);
  const right = shown.slice(2);

  return (
    <nav
      aria-label={recordLabel}
      data-testid="bottom-bar"
      className="border-line bg-bg-soft relative flex shrink-0 items-stretch border-t pb-[env(safe-area-inset-bottom)]"
    >
      {left.map((item) => (
        <Tab key={item.key} item={item} active={active === item.key} onNavigate={onNavigate} />
      ))}

      {/* The record button, in the middle and raised.

          Centre because it is the one action the app exists for, and raised because a target that
          sits in the row is a target the thumb has to aim at rather than land on. */}
      <div className="relative w-[76px] shrink-0">
        <m.button
          type="button"
          onClick={onRecord}
          aria-pressed={recording}
          aria-label={recordLabel}
          whileTap={{ scale: 0.92 }}
          transition={SNAPPY}
          className={cn(
            "absolute -top-5 left-1/2 grid size-14 -translate-x-1/2 place-items-center rounded-full border-2 shadow-[var(--shadow-pop)]",
            recording ? "border-rec bg-rec-soft" : "border-line-strong bg-bg-elevated",
          )}
        >
          <span
            className={cn(
              "bg-rec block transition-all duration-200",
              recording ? "size-4 rounded-sm" : "size-6 rounded-full",
            )}
          />
        </m.button>
      </div>

      {right.map((item) => (
        <Tab key={item.key} item={item} active={active === item.key} onNavigate={onNavigate} />
      ))}
    </nav>
  );
}

function Tab({
  item,
  active,
  onNavigate,
}: {
  item: NavItem;
  active: boolean;
  onNavigate: (key: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onNavigate(item.key)}
      aria-current={active ? "page" : undefined}
      // `min-h-14`: a thumb target, not a mouse target. Anything under about 44px is a tap that
      // lands next to what it aimed at.
      className={cn(
        "flex min-h-14 flex-1 flex-col items-center justify-center gap-0.5 px-1 py-2 transition-colors",
        active ? "text-accent" : "text-fg-faint",
      )}
    >
      <item.icon aria-hidden="true" className="size-5 shrink-0 stroke-[1.75]" />
      <span className="text-micro w-full truncate text-center">{item.label}</span>
    </button>
  );
}
