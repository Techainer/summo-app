import type { ReactNode } from "react";

import { useT } from "../../i18n/context";
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

/**
 * Wording is a translation key, resolved at render.
 *
 * A module-level table of Vietnamese strings meant every chip in the app said "Chưa làm" whatever
 * language the interface was in — and this component is used on the task board, the agent runs and
 * the import queue, so it was most of the words on three screens.
 */
const LABELS: Record<Status, string> = {
  todo: "tasks.todo",
  running: "tasks.running",
  done: "tasks.done",
  blocked: "tasks.blocked",
  failed: "tasks.failed",
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
  const t = useT();
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1",
        "text-micro font-semibold tracking-wide uppercase",
        STYLES[status],
        className,
      )}
    >
      {ICONS[status]}
      {label ?? t(LABELS[status])}
    </span>
  );
}
