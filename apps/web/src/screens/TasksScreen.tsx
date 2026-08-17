import { Bot, CheckCircle2, Circle, ListChecks } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import {
  Avatar,
  Button,
  Card,
  CardBody,
  CardHeader,
  Empty,
  EmptyColumn,
  Page,
  SegmentedControl,
  StatusChip,
} from "../components/ui";
import { useErrorText } from "../lib/errors";
import { cn } from "../lib/cn";
import { useT } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { today } from "../lib/report";
import { useRefresh } from "../lib/use-load";
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

// Labels are keys, resolved at render — see the note in AnalyticsScreen.
const VIEWS = [
  { value: "people" as const, labelKey: "tasks.mine" },
  { value: "agent" as const, labelKey: "tasks.agent" },
];

/**
 * Two boards, because there are two kinds of work.
 *
 * A person's task moves between columns; somebody decides it is done. An agent's task moves through
 * a list of steps the agent wrote for itself, and finishes when the last one does. Drawing them the
 * same way would invite the user to drag an agent task to "Xong", which is not how it gets there.
 */
export function TasksScreen() {
  const t = useT();
  const say = useErrorText();
  const { handshake } = useEngine();
  const client = useMemo(() => new TaskClient(handshake), [handshake]);
  const [board, setBoard] = useState<Board | null>(null);
  const [view, setView] = useState<View>("people");
  const [owner, setOwner] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [runningId, setRunningId] = useState<string | null>(null);
  const now = today();

  const load = useCallback(async () => {
    try {
      setBoard(await client.board());
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useRefresh(load);

  const move = useCallback(
    async (id: string, status: ColumnStatus) => {
      // Optimistic: the write goes to a local file and comes back in milliseconds, so waiting for
      // it before redrawing makes dragging feel broken. A failure reloads the truth.
      setBoard((current) => (current ? shift(current, id, status) : current));
      try {
        await client.move(id, { status });
      } catch (e) {
        setError(say(e));
        void load();
      }
    },
    [client, load, say],
  );

  const start = useCallback(
    async (id: string) => {
      setRunningId(id);
      try {
        await client.run(id);
      } catch (e) {
        setError(say(e));
      } finally {
        setRunningId(null);
        // The run wrote its steps to the vault; re-read rather than guessing what changed.
        void load();
      }
    },
    [client, load, say],
  );

  if (error && !board) {
    return (
      <div className="p-5">
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
          {error}
        </p>
      </div>
    );
  }
  if (!board)
    return (
      <p className="text-fg-faint grid h-full place-items-center text-center">
        {t("tasks.opening")}
      </p>
    );

  return (
    // A column that fills the pane, so the board below the header can be told to take what is left.
    // Four kanban lanes 180px tall with 300px of background under them read as a screen that failed
    // to load; a lane is a place you drop things into and it should look like one.
    <Page
      fill
      title={t("tasks.heading")}
      actions={
        <SegmentedControl
          label={t("tasks.kind")}
          size="sm"
          options={VIEWS.map((v) => ({ value: v.value, label: t(v.labelKey) }))}
          value={view}
          onChange={setView}
        />
      }
    >
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta mt-3 rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {view === "people" ? (
        <div className="flex min-h-0 flex-1 flex-col">
          {board.owners.length > 0 && (
            <div className="mt-3 flex flex-wrap items-center gap-1.5">
              <FilterChip
                label={t("tasks.all")}
                on={owner === null}
                onClick={() => setOwner(null)}
              />
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

          {/* Four empty columns is not "no work"; it is a screen that failed to load, which is what
              a board with nothing on it looked like: six hundred pixels of bordered grey. A board
              only draws its columns once there is something to put in one. */}
          {COLUMNS.every((status) => forOwner(board[status], owner).length === 0) ? (
            <Empty
              full
              icon={ListChecks}
              sticker="party"
              title={t("tasks.board_empty")}
              hint={t("tasks.board_empty_hint")}
            />
          ) : (
            <div className="mt-4 grid min-h-0 flex-1 gap-3 md:grid-cols-2 xl:grid-cols-4">
              {COLUMNS.map((status) => {
                const items = forOwner(board[status], owner);
                return (
                  <Column
                    key={status}
                    label={t(`tasks.${status}`)}
                    count={items.length}
                    onDrop={(id) => void move(id, status)}
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
          )}
        </div>
      ) : (
        <div className="mt-4 min-h-0 flex-1 space-y-3 overflow-y-auto">
          {board.agent.length === 0 ? (
            <Empty
              full
              icon={Bot}
              sticker="robot"
              title={t("tasks.agent_empty_head")}
              hint={t("tasks.agent_empty_tail")}
            />
          ) : (
            board.agent.map((task) => (
              <AgentCard
                key={task.id}
                task={task}
                running={runningId === task.id}
                onRun={() => void start(task.id)}
              />
            ))
          )}
        </div>
      )}
    </Page>
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
        "text-meta rounded-full border px-2.5 py-1 transition-colors",
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
  const t = useT();
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
        "flex min-h-0 flex-col rounded-[var(--radius-card)] border p-2 transition-colors",
        over ? "border-accent/50 bg-accent-soft" : "border-line bg-bg-soft/40",
      )}
    >
      <h2 className="text-fg-faint text-micro px-1 pb-2 font-semibold tracking-wider uppercase">
        {label}
        <span className="nums ml-1.5 font-normal">{count}</span>
      </h2>
      {/* An empty column says so rather than being a bordered rectangle of nothing. Four of those
          side by side is what made a board with no work on it look like a board that failed to
          load. */}
      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto">
        {count === 0 ? <EmptyColumn>{t("empty.column")}</EmptyColumn> : children}
      </div>
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
  const t = useT();
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
        "border-line bg-bg-raised cursor-grab rounded-[var(--radius-card)] border p-2.5",
        "transition-all duration-150 hover:-translate-y-0.5 hover:shadow-[var(--shadow-card)]",
        "active:cursor-grabbing",
        // Held, not hovered: a card under the pointer lifts a little, one being dragged lifts
        // further and dims, so the gap it left reads as a gap rather than as a deleted row.
        dragging && "scale-[1.02] opacity-50 shadow-[var(--shadow-pop)]",
      )}
    >
      <p
        className={cn(
          "text-sm leading-snug",
          task.status === "done" && "text-fg-faint line-through",
        )}
      >
        {task.text}
      </p>
      <div className="text-micro mt-1.5 flex flex-wrap items-center gap-2">
        {/* The disc first, so a column of cards can be scanned for one person's work without
            reading a single name. */}
        {task.owner && (
          <span className="text-fg-dim flex items-center gap-1.5">
            <Avatar name={task.owner} size="sm" />@{task.owner}
          </span>
        )}
        {task.due && (
          <span className={cn("nums", overdue ? "text-rec" : "text-fg-faint")}>
            {((d) => t(d.key, d.params))(dueLabel(task.due, now))}
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
function AgentCard({ task, running, onRun }: { task: Task; running: boolean; onRun: () => void }) {
  const t = useT();
  const [open, setOpen] = useState(task.status === "doing");
  const progress = stepProgress(task);
  const step = currentStep(task);

  return (
    <Card>
      <CardHeader
        title={task.text}
        count={progress === null ? undefined : `${progress}%`}
        actions={
          <>
            <StatusChip status={running ? "running" : mapStatus(task.status)} />
            {task.status !== "done" && (
              <Button size="sm" variant="primary" busy={running} onClick={onRun}>
                {t("tasks.run")}
              </Button>
            )}
          </>
        }
      />
      <CardBody>
        {step && task.status === "doing" && <p className="text-running text-meta">◐ {step.text}</p>}

        {(task.steps?.length ?? 0) > 0 && (
          <>
            <button
              type="button"
              onClick={() => setOpen((o) => !o)}
              aria-expanded={open}
              className="text-fg-dim hover:text-fg text-meta mt-2 font-medium underline-offset-2 hover:underline"
            >
              {open
                ? t("tasks.hide_steps")
                : t("tasks.show_steps_n", { count: task.steps?.length ?? 0 })}
            </button>
            {open && (
              <ul className="mt-2 space-y-1">
                {task.steps?.map((s, i) => (
                  <li
                    key={`${s.text}-${i}`}
                    className={cn(
                      "text-meta flex items-baseline gap-2",
                      s.done ? "text-fg-faint" : "text-fg",
                    )}
                  >
                    {s.done ? (
                      <CheckCircle2 aria-hidden="true" className="text-done size-3.5 shrink-0" />
                    ) : (
                      <Circle aria-hidden="true" className="text-fg-faint size-3.5 shrink-0" />
                    )}
                    {s.text}
                  </li>
                ))}
              </ul>
            )}
          </>
        )}

        {(task.steps?.length ?? 0) === 0 && (
          <p className="text-fg-faint text-meta">{t("tasks.no_plan")}</p>
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
