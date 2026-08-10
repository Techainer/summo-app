import { useCallback, useEffect, useMemo, useState } from "react";

import { Card, CardBody, CardHeader, SegmentedControl, StatusChip } from "../components/ui";
import { cn } from "../lib/cn";
import { useEngine } from "../lib/engine-context";
import { today } from "../lib/report";
import {
  COLUMNS,
  TaskClient,
  currentStep,
  dueLabel,
  forOwner,
  isOverdue,
  stepProgress,
  type Board,
  type ColumnStatus,
  type Status,
  type Task,
} from "../lib/tasks";

type View = "people" | "agent";

const VIEWS = [
  { value: "people" as const, label: "Của mọi người" },
  { value: "agent" as const, label: "Của agent" },
];

/**
 * Two boards, because there are two kinds of work.
 *
 * A person's task moves between columns; somebody decides it is done. An agent's task moves through
 * a list of steps the agent wrote for itself, and finishes when the last one does. Drawing them the
 * same way would invite the user to drag an agent task to "Xong", which is not how it gets there.
 */
export function TasksScreen() {
  const { handshake } = useEngine();
  const client = useMemo(() => new TaskClient(handshake), [handshake]);
  const [board, setBoard] = useState<Board | null>(null);
  const [view, setView] = useState<View>("people");
  const [owner, setOwner] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const now = today();

  const load = useCallback(async () => {
    try {
      setBoard(await client.board());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void load();
  }, [load]);

  const move = useCallback(
    async (id: string, status: ColumnStatus) => {
      // Optimistic: the write goes to a local file and comes back in milliseconds, so waiting for
      // it before redrawing makes dragging feel broken. A failure reloads the truth.
      setBoard((current) => (current ? shift(current, id, status) : current));
      try {
        await client.move(id, { status });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        void load();
      }
    },
    [client, load],
  );

  if (error && !board) {
    return (
      <div className="p-5">
        <p className="rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
          {error}
        </p>
      </div>
    );
  }
  if (!board) return <p className="mt-24 text-center text-fg-faint">Đang mở…</p>;

  return (
    <div className="p-5">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold tracking-tight">Việc cần làm</h1>
        <SegmentedControl
          className="ml-auto"
          label="Loại việc"
          size="sm"
          options={VIEWS}
          value={view}
          onChange={setView}
        />
      </div>

      {error && (
        <p className="mt-3 rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
          {error}
        </p>
      )}

      {view === "people" ? (
        <>
          {board.owners.length > 0 && (
            <div className="mt-3 flex flex-wrap items-center gap-1.5">
              <FilterChip label="Tất cả" on={owner === null} onClick={() => setOwner(null)} />
              {board.owners.map((name) => (
                <FilterChip
                  key={name}
                  label={name}
                  on={owner === name}
                  onClick={() => setOwner(name)}
                />
              ))}
            </div>
          )}

          <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            {COLUMNS.map((column) => {
              const items = forOwner(board[column.status], owner);
              return (
                <Column
                  key={column.status}
                  label={column.label}
                  count={items.length}
                  onDrop={(id) => void move(id, column.status)}
                >
                  {items.map((task) => (
                    <PersonCard
                      key={task.id}
                      task={task}
                      today={now}
                      dragging={dragging === task.id}
                      onDragStart={() => setDragging(task.id)}
                      onDragEnd={() => setDragging(null)}
                    />
                  ))}
                </Column>
              );
            })}
          </div>
        </>
      ) : (
        <div className="mt-4 space-y-3">
          {board.agent.length === 0 ? (
            <p className="mt-16 text-center text-fg-faint">
              Agent chưa có việc nào. Giao việc bằng cách viết{" "}
              <code className="tabular text-[13px]">- [ ] @agent …</code> trong ghi chú buổi họp.
            </p>
          ) : (
            board.agent.map((task) => <AgentCard key={task.id} task={task} />)
          )}
        </div>
      )}
    </div>
  );
}

/** Move a task between columns in the local copy, so a drag redraws immediately. */
function shift(board: Board, id: string, to: ColumnStatus): Board {
  const columns: ColumnStatus[] = ["todo", "doing", "done", "blocked"];
  let moved: Task | undefined;
  const next: Board = { ...board };
  for (const key of columns) {
    const found = board[key].find((t) => t.id === id);
    if (found) moved = { ...found, status: to };
    next[key] = board[key].filter((t) => t.id !== id);
  }
  if (!moved) return board;
  next[to] = [...next[to], moved];
  return next;
}

function FilterChip({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      className={cn(
        "rounded-full border px-2.5 py-1 text-[13px] transition-colors",
        on
          ? "border-accent/40 bg-accent-soft text-accent"
          : "border-line text-fg-dim hover:text-fg",
      )}
    >
      {label}
    </button>
  );
}

function Column({
  label,
  count,
  onDrop,
  children,
}: {
  label: string;
  count: number;
  onDrop: (id: string) => void;
  children: React.ReactNode;
}) {
  const [over, setOver] = useState(false);
  return (
    <section
      onDragOver={(e) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        const id = e.dataTransfer.getData("text/plain");
        if (id) onDrop(id);
      }}
      aria-label={label}
      className={cn(
        "rounded-[var(--radius-card)] border p-2 transition-colors",
        over ? "border-accent/50 bg-accent-soft" : "border-line bg-bg-soft/40",
      )}
    >
      <h2 className="px-1 pb-2 text-[12px] font-semibold uppercase tracking-wider text-fg-faint">
        {label}
        <span className="tabular ml-1.5 font-normal">{count}</span>
      </h2>
      <div className="space-y-2">{children}</div>
    </section>
  );
}

function PersonCard({
  task,
  today: now,
  dragging,
  onDragStart,
  onDragEnd,
}: {
  task: Task;
  today: string;
  dragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
}) {
  const overdue = isOverdue(task, now);
  return (
    <article
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/plain", task.id);
        onDragStart();
      }}
      onDragEnd={onDragEnd}
      className={cn(
        "cursor-grab rounded-[var(--radius-card)] border border-line bg-bg-raised p-2.5",
        "active:cursor-grabbing",
        dragging && "opacity-50",
      )}
    >
      <p className={cn("text-sm leading-snug", task.status === "done" && "text-fg-faint line-through")}>
        {task.text}
      </p>
      <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[12px]">
        {task.owner && <span className="text-fg-dim">@{task.owner}</span>}
        {task.due && (
          <span className={cn("tabular", overdue ? "text-rec" : "text-fg-faint")}>
            {dueLabel(task.due, now)}
          </span>
        )}
      </div>
    </article>
  );
}

/**
 * An agent task, with the plan it wrote for itself.
 *
 * The step list is the point: it is what makes an autonomous task legible instead of a spinner. A
 * user who can see "đã quét ghi chú → đang soạn sự kiện" knows both what happened and what to blame
 * when the result is wrong.
 */
function AgentCard({ task }: { task: Task }) {
  const [open, setOpen] = useState(task.status === "doing");
  const progress = stepProgress(task);
  const step = currentStep(task);

  return (
    <Card>
      <CardHeader
        title={task.text}
        count={progress === null ? undefined : `${progress}%`}
        actions={<StatusChip status={mapStatus(task.status)} />}
      />
      <CardBody>
        {step && task.status === "doing" && (
          <p className="text-[13px] text-running">◐ {step.text}</p>
        )}

        {(task.steps?.length ?? 0) > 0 && (
          <>
            <button
              type="button"
              onClick={() => setOpen((o) => !o)}
              aria-expanded={open}
              className="mt-2 text-[13px] font-medium text-fg-dim underline-offset-2 hover:text-fg hover:underline"
            >
              {open ? "Ẩn các bước" : `Xem ${task.steps?.length} bước`}
            </button>
            {open && (
              <ul className="mt-2 space-y-1">
                {task.steps?.map((s, i) => (
                  <li
                    key={`${s.text}-${i}`}
                    className={cn(
                      "flex items-baseline gap-2 text-[13px]",
                      s.done ? "text-fg-faint" : "text-fg",
                    )}
                  >
                    <span aria-hidden="true">{s.done ? "✓" : "○"}</span>
                    {s.text}
                  </li>
                ))}
              </ul>
            )}
          </>
        )}

        {(task.steps?.length ?? 0) === 0 && (
          <p className="text-[13px] text-fg-faint">Agent chưa lập kế hoạch cho việc này.</p>
        )}
      </CardBody>
    </Card>
  );
}

function mapStatus(status: Status) {
  switch (status) {
    case "doing":
      return "running" as const;
    case "done":
      return "done" as const;
    case "blocked":
      return "blocked" as const;
    case "failed":
      return "failed" as const;
    default:
      return "todo" as const;
  }
}
