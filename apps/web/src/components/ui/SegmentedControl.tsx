import * as ToggleGroup from "@radix-ui/react-toggle-group";
import { motion } from "motion/react";
import { useId } from "react";

import { SPRING } from "../../lib/motion";
import { cn } from "../../lib/cn";

export interface Segment<T extends string> {
  value: T;
  label: string;
}

interface Props<T extends string> {
  options: Segment<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Accessible name for the group, e.g. "Nguồn ghi" or "Chế độ xem". */
  label: string;
  size?: "sm" | "md";
  className?: string;
}

/**
 * A two-or-three-way choice where the options are peers.
 *
 * `[Record | Upload]` and `[Notes | Transcript]` in the reference designs are both this. It is a
 * different thing from tabs: tabs navigate between panels, this sets a value.
 *
 * The selected background is one shared element that slides between options rather than a
 * background per option that fades — with the latter, moving between two adjacent options reads as
 * two separate things happening instead of one thing moving.
 */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  label,
  size = "md",
  className,
}: Props<T>) {
  const layoutId = useId();

  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      // Radix reports "" when the pressed item is toggled off; a segmented control has no off.
      onValueChange={(next) => next && onChange(next as T)}
      aria-label={label}
      className={cn(
        "bg-bg-soft border-line inline-flex items-center rounded-full border p-0.5",
        className,
      )}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <ToggleGroup.Item
            key={option.value}
            value={option.value}
            className={cn(
              "relative rounded-full font-medium transition-colors",
              size === "sm" ? "h-7 px-3 text-[13px]" : "h-8 px-4 text-sm",
              selected ? "text-accent-fg" : "text-fg-dim hover:text-fg",
            )}
          >
            {selected && (
              <motion.span
                layoutId={layoutId}
                className="bg-accent absolute inset-0 rounded-full"
                transition={SPRING}
              />
            )}
            <span className="relative">{option.label}</span>
          </ToggleGroup.Item>
        );
      })}
    </ToggleGroup.Root>
  );
}
