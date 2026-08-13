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
    <div className={cn("flex items-center gap-3 px-5 pt-4 pb-3", className)}>
      <h2 className="text-body font-semibold tracking-tight">
        {title}
        {count !== undefined && <span className="text-fg-faint ml-2 font-normal">· {count}</span>}
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
