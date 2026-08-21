import type { HTMLAttributes, ReactNode } from "react";

import { cn } from "../../lib/cn";

export function Card({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "border-line bg-bg-raised rounded-[var(--radius-card)] border shadow-[var(--shadow-card)]",
        className,
      )}
      {...props}
    />
  );
}

/**
 * A card's title row, with room for controls on the right.
 *
 * `count` is separated from `title` because "Việc cần làm · 4 việc" reads as one heading but the
 * number changes far more often than the words, and keeping them apart avoids re-rendering a
 * string every time a task completes.
 */
export function CardHeader({
  title,
  count,
  actions,
  className,
}: {
  title: ReactNode;
  count?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    /* Wraps rather than squeezes.
     *
     * `min-w-0` on the heading below lets it shrink to nothing so the actions always fit, which is
     * right until the actions are two buttons and the title is a Vietnamese sentence: then the
     * heading is squeezed to about half the card and breaks mid-phrase, and the count breaks with
     * it — "Bản tóm tắt / agent viết" beside "· chưa / duyệt" on the draft panel, in the very
     * screenshot the landing page was built from. Allowing the row to wrap puts the buttons on a
     * line of their own instead, which costs one row on a card that had the space anyway. */
    <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-2 px-5 pt-4 pb-3", className)}>
      {/* The heading is a flex row, because a title is often an icon beside a word — and a
          `<span class="flex">` inside a plain `<h2>` is a block, which pushed the count onto a line
          of its own. Every card with an icon in its title was reading "Đang chờ bạn" and then, on
          the next line, "· 1". */}
      <h2 className="text-body flex min-w-0 items-center gap-2 font-semibold tracking-tight">
        {title}
        {/* One phrase, so it breaks as one thing or not at all. "· chưa duyệt" splitting across
            two lines reads as two separate marks rather than as a status. */}
        {count !== undefined && (
          <span className="text-fg-faint font-normal whitespace-nowrap">· {count}</span>
        )}
      </h2>
      {actions && <div className="ml-auto flex items-center gap-1.5">{actions}</div>}
    </div>
  );
}

export function CardBody({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  // 20px sides, matching the header above it, and 20px at the foot. A card whose body is inset
  // less than its title is a card whose text does not line up with its own heading, which is
  // visible on every screen at once and nameable on none of them.
  return <div className={cn("px-5 pb-5", className)} {...props} />;
}
