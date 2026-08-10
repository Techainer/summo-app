import type { ReactNode } from "react";

import { cn } from "../../lib/cn";
import { Spinner } from "./Button";

/**
 * States a task or a job can be in.
 *
 * `blocked` and `failed` are separate because they need different actions from the user: blocked
 * waits on something, failed needs a decision.
 */
export type Status = "todo" | "running" | "done" | "blocked" | "failed";

const STYLES: Record<Status, string> = {
  todo: "bg-bg-soft text-fg-faint border-line",
  running: "bg-running-soft text-running border-running/30",
  done: "bg-accent-soft text-done border-accent/30",
  blocked: "bg-blocked-soft text-blocked border-blocked/30",
  failed: "bg-rec-soft text-rec border-rec/30",
};

/** Vietnamese first: this is a Vietnamese product and the interface should not read as translated. */
const LABELS: Record<Status, string> = {
  todo: "Chưa làm",
  running: "Đang chạy",
  done: "Xong",
  blocked: "Đang chờ",
  failed: "Lỗi",
};

const ICONS: Record<Status, ReactNode> = {
  todo: null,
  running: <Spinner className="size-3 border-[1.5px]" />,
  done: <span aria-hidden="true">✓</span>,
  blocked: <span aria-hidden="true">◷</span>,
  failed: <span aria-hidden="true">!</span>,
};

export function StatusChip({
  status,
  label,
  className,
}: {
  status: Status;
  /** Override the default wording, e.g. to show a step count. */
  label?: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1",
        "text-[11px] font-semibold uppercase tracking-wide",
        STYLES[status],
        className,
      )}
    >
      {ICONS[status]}
      {label ?? LABELS[status]}
    </span>
  );
}
