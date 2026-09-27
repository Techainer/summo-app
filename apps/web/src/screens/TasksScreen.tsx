import { Bot, CheckCircle2, Circle, ListChecks, Pencil, Plus, Trash2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import {
  Alert,
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

/**
 * Which board, and in which shape.
 *
 * `list` is new and is the answer to *"sao không kanban + giao diện kéo thả dễ hiểu (Multi view)"*
 * — the kanban was already here and dragging already worked; what was missing was a second way to
 * look at the same tasks. Four columns are good for moving work along and bad for reading it in
 * order, and a phone shows one of them at a time.
 */
type View = "people" | "list" | "agent";

// Labels are keys, resolved at render — see the note in AnalyticsScreen.
const VIEWS = [
  { value: "people" as const, labelKey: "tasks.board" },
  { value: "list" as const, labelKey: "tasks.list" },
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

  /** Write one down that came out of no meeting. */
  const add = useCallback(
    async (text: string, owner?: string, due?: string) => {
      try {
        await client.add(text, owner, due);
      } catch (e) {
        setError(say(e));
      } finally {
        void load();
      }
    },
    [client, load, say],
  );

  /** Reword one, or change who owns it and when it is due. */
  const edit = useCallback(
    async (id: string, patch: { text?: string; owner?: string | null; due?: string | null }) => {
      try {
        await client.move(id, patch);
      } catch (e) {
        setError(say(e));
      } finally {
        void load();
      }
    },
    [client, load, say],
  );

  /**
   * Take one off, rather than dragging a thing that was never a task to "done".
   *
   * Optimistic like `move`, and for the same reason: the write goes to a local file. A failure
   * reloads the truth, so a refused delete puts the card back rather than losing it from the
   * screen while it is still on disk.
   */
  const drop = useCallback(
    async (id: string) => {
      setBoard((current) => (current ? without(current, id) : current));
      try {
        await client.remove(id);
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
        <Alert tone="rec">{error}</Alert>
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
        <Alert tone="rec" className="mt-3">
          {error}
        </Alert>
      )}

      {view !== "agent" && <Composer onAdd={(text, owner, due) => void add(text, owner, due)} />}

      {view === "people" || view === "list" ? (
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
          ) : view === "list" ? (
            /* The same tasks, in one column, in the order somebody reads them: what is open
               first, then by how soon it is due. A board is for moving work along; a list is for
               finding out what there is. */
            <ul className="mt-4 min-h-0 flex-1 space-y-1.5 overflow-y-auto">
              {COLUMNS.flatMap((status) => forOwner(board[status], owner))
                .sort(byUrgency)
                .map((task) => (
                  <TaskRow
                    key={task.id}
                    task={task}
                    today={now}
                    onStatus={(status) => void move(task.id, status)}
                    onEdit={(patch) => void edit(task.id, patch)}
                    onDelete={() => void drop(task.id)}
                  />
                ))}
            </ul>
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
                        onEdit={(text) => void edit(task.id, { text })}
                        onDelete={() => void drop(task.id)}
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

/**
 * The order a list is read in: open work first, then by how soon it is due.
 *
 * A dateless task sorts after a dated one rather than before it — "no date" is not "due never",
 * but it is certainly not more urgent than something due on Friday.
 */
function byUrgency(a: Task, b: Task): number {
  const rank = (task: Task) => (task.status === "done" ? 1 : 0);
  if (rank(a) !== rank(b)) return rank(a) - rank(b);
  if (a.due !== b.due) return (a.due ?? "\uffff").localeCompare(b.due ?? "\uffff");
  return a.text.localeCompare(b.text);
}

/** Take a task out of the local copy, so a delete redraws immediately. */
function without(board: Board, id: string): Board {
  const next: Board = { ...board };
  for (const key of ["todo", "doing", "done", "blocked", "agent"] as const) {
    next[key] = board[key].filter((t) => t.id !== id);
  }
  return next;
}

/**
 * Write down a task that no meeting produced.
 *
 * Owner and due date are beside the text rather than behind a dialogue: both are one short field,
 * and a form that makes you open something to say "@binh, Friday" is a form people stop using.
 */
function Composer({ onAdd }: { onAdd: (text: string, owner?: string, due?: string) => void }) {
  const t = useT();
  const [text, setText] = useState("");
  const [owner, setOwner] = useState("");
  const [due, setDue] = useState("");

  const submit = () => {
    if (!text.trim()) return;
    onAdd(text.trim(), owner.trim() || undefined, due || undefined);
    setText("");
    setOwner("");
    setDue("");
  };

  return (
    <form
      className="border-line bg-bg-soft/40 rounded-card mt-3 flex flex-wrap items-center gap-2 border p-2"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <input
        className="border-line bg-bg-raised text-fg focus-visible:border-accent rounded-control text-body min-w-48 flex-1 border px-2.5 py-1.5 focus:outline-none"
        value={text}
        aria-label={t("tasks.add")}
        placeholder={t("tasks.new_placeholder")}
        onChange={(e) => setText(e.target.value)}
      />
      <input
        className="border-line bg-bg-raised text-fg focus-visible:border-accent rounded-control text-meta w-28 border px-2 py-1.5 focus:outline-none"
        value={owner}
        aria-label={t("tasks.owner_placeholder")}
        placeholder={t("tasks.owner_placeholder")}
        onChange={(e) => setOwner(e.target.value)}
      />
      <input
        className="border-line bg-bg-raised text-fg focus-visible:border-accent rounded-control text-meta nums border px-2 py-1.5 focus:outline-none"
        type="date"
        value={due}
        aria-label={t("tasks.due_on", { date: "" })}
        onChange={(e) => setDue(e.target.value)}
      />
      <Button type="submit" size="sm" variant="primary" disabled={!text.trim()}>
        <Plus aria-hidden="true" className="size-3.5" />
        {t("tasks.add")}
      </Button>
    </form>
  );
}

/**
 * Reword, and delete.
 *
 * Both were missing everywhere: a task read out of a summary is sometimes half a sentence, and a
 * line that was never a task could only be dragged to "Done" — a lie in the one place somebody
 * looks to find out what they finished.
 *
 * Delete asks first. It rewrites a file in the user's own vault, and unlike moving a column there
 * is nothing to drag back.
 */
function TaskTools({
  text,
  onEdit,
  onDelete,
}: {
  text: string;
  onEdit: (text: string) => void;
  onDelete: () => void;
}) {
  const t = useT();
  const [editing, setEditing] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  if (editing !== null) {
    return (
      <form
        className="flex flex-1 items-center gap-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          if (editing.trim()) onEdit(editing.trim());
          setEditing(null);
        }}
      >
        <input
          autoFocus
          className="border-line bg-bg-raised text-fg focus-visible:border-accent rounded-control text-body min-w-0 flex-1 border px-2 py-1 focus:outline-none"
          value={editing}
          aria-label={t("tasks.edit")}
          onChange={(e) => setEditing(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setEditing(null)}
        />
        <Button type="submit" size="sm" variant="primary" disabled={!editing.trim()}>
          {t("common.save")}
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={() => setEditing(null)}>
          {t("common.cancel")}
        </Button>
      </form>
    );
  }

  if (confirming) {
    return (
      <span className="text-fg-dim text-micro flex items-center gap-1.5">
        <Button size="sm" variant="danger" onClick={onDelete}>
          {t("common.delete")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
          {t("common.cancel")}
        </Button>
      </span>
    );
  }

  return (
    <span className="ms-auto flex items-center gap-0.5">
      <button
        type="button"
        onClick={() => setEditing(text)}
        aria-label={t("tasks.edit")}
        className="text-fg-faint hover:text-fg hover:bg-bg-soft rounded-control p-1"
      >
        <Pencil aria-hidden="true" className="size-3.5" />
      </button>
      <button
        type="button"
        onClick={() => setConfirming(true)}
        aria-label={t("tasks.delete_task")}
        className="text-fg-faint hover:text-rec hover:bg-bg-soft rounded-control p-1"
      >
        <Trash2 aria-hidden="true" className="size-3.5" />
      </button>
    </span>
  );
}

/** One task on the list, where the column is a control rather than a place. */
function TaskRow({
  task,
  today: now,
  onStatus,
  onEdit,
  onDelete,
}: {
  task: Task;
  today: string;
  onStatus: (status: ColumnStatus) => void;
  onEdit: (patch: { text?: string; owner?: string | null; due?: string | null }) => void;
  onDelete: () => void;
}) {
  const t = useT();
  const overdue = isOverdue(task, now);
  return (
    <li className="border-line bg-bg-raised rounded-card flex flex-wrap items-center gap-2 border px-2.5 py-2">
      <select
        value={task.status === "failed" ? "blocked" : task.status}
        aria-label={t("tasks.kind")}
        onChange={(e) => onStatus(e.target.value as ColumnStatus)}
        className="border-line bg-bg-soft text-fg-dim rounded-control text-micro border px-1.5 py-1"
      >
        {COLUMNS.map((status) => (
          <option key={status} value={status}>
            {t(`tasks.${status}`)}
          </option>
        ))}
      </select>
      <span
        className={cn(
          "text-body min-w-0 flex-1",
          task.status === "done" && "text-fg-faint line-through",
        )}
      >
        {task.text}
      </span>
      {task.owner && (
        <span className="text-fg-dim text-micro flex items-center gap-1.5">
          <Avatar name={task.owner} size="sm" />@{task.owner}
        </span>
      )}
      {task.due && (
        <span className={cn("nums text-micro", overdue ? "text-rec" : "text-fg-faint")}>
          {((d) => t(d.key, d.params))(dueLabel(task.due, now))}
        </span>
      )}
      <TaskTools text={task.text} onEdit={(text) => onEdit({ text })} onDelete={onDelete} />
    </li>
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
        "rounded-card flex min-h-0 flex-col border p-2 transition-colors",
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
  onEdit,
  onDelete,
}: {
  task: Task;
  today: string;
  dragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  onEdit: (text: string) => void;
  onDelete: () => void;
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
        "border-line bg-bg-raised rounded-card cursor-grab border p-2.5",
        "transition-all duration-[var(--motion-hover)] hover:-translate-y-0.5 hover:shadow-[var(--shadow-card)]",
        "active:cursor-grabbing",
        // Held, not hovered: a card under the pointer lifts a little, one being dragged lifts
        // further and dims, so the gap it left reads as a gap rather than as a deleted row.
        dragging && "scale-[1.02] opacity-50 shadow-[var(--shadow-pop)]",
      )}
    >
      <p
        className={cn(
          "text-body leading-snug",
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
        <TaskTools text={task.text} onEdit={onEdit} onDelete={onDelete} />
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
