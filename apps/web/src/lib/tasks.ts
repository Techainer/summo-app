import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * The board.
 *
 * Two kinds of work share one format but not one board. A person's task moves between three
 * columns; the agent's task moves through a list of steps it wrote itself. Rendering them the same
 * way would imply the user drags an agent task to "Done", which is not how it finishes.
 */

export type Status = "todo" | "doing" | "done" | "blocked" | "failed";

export interface Step {
  text: string;
  done: boolean;
}

export interface Task {
  id: string;
  text: string;
  owner?: string;
  status: Status;
  due?: string;
  steps?: Step[];
  file: string;
  line: number;
}

export interface Board {
  todo: Task[];
  doing: Task[];
  done: Task[];
  blocked: Task[];
  agent: Task[];
  owners: string[];
}

/**
 * The columns a person's task can sit in.
 *
 * Narrower than `Status` on purpose: `failed` is an agent outcome and shares the "waiting on a
 * human" column with `blocked`, so there is no column to drag a task into that produces it.
 */
export type ColumnStatus = "todo" | "doing" | "done" | "blocked";

/**
 * Columns a person's task can be dragged between, in the order they are drawn.
 *
 * Statuses only. The heading each one shows is `tasks.<status>` resolved at render — these used to
 * carry their own Vietnamese labels, so the English interface drew a board headed "CHƯA LÀM / ĐANG
 * LÀM / ĐANG CHỜ / XONG" under an English title, and a language somebody added by dropping in a
 * JSON file could never have reached this line at all.
 */
export const COLUMNS: ColumnStatus[] = ["todo", "doing", "blocked", "done"];

export class TaskClient {
  constructor(private readonly handshake: Handshake) {}

  async board(): Promise<Board> {
    return readJson<Board>(await fetch(url(this.handshake, "/tasks")));
  }

  /**
   * Change a task. Omitted fields are left alone; an explicit `null` owner or due date clears it,
   * which is why they are distinguished rather than collapsed into "falsy".
   */
  async move(
    id: string,
    patch: { status?: Status; owner?: string | null; due?: string | null },
  ): Promise<Task> {
    return readJson<Task>(
      await fetch(url(this.handshake, `/tasks/${encodeURIComponent(id)}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(patch),
      }),
    );
  }

  /**
   * Hand an agent task to the agent and wait for it.
   *
   * The steps land in the vault as they happen, so a client that navigates away still finds the
   * trace when it comes back — this promise resolving is the end, not the only signal.
   */
  run(id: string): Promise<{ task: string; status: Status; outcome: string; steps: Step[] }> {
    return fetch(url(this.handshake, `/tasks/${encodeURIComponent(id)}/run`), {
      method: "POST",
    }).then(
      readJson<{
        task: string;
        status: Status;
        outcome: string;
        steps: Step[];
      }>,
    );
  }

  async create(meeting: string, text: string, owner?: string, due?: string): Promise<Task> {
    return readJson<Task>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/tasks`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ text, owner, due }),
      }),
    );
  }
}

/** Tasks in a column, optionally narrowed to one owner. */
export function forOwner(tasks: Task[], owner: string | null): Task[] {
  return owner === null ? tasks : tasks.filter((t) => t.owner === owner);
}

/**
 * How far the agent has got through its own breakdown, as a percentage.
 *
 * Returns null rather than 0 when there are no steps, because "no steps" and "no progress" mean
 * different things: the first is a task the agent has not planned yet.
 */
export function stepProgress(task: Task): number | null {
  const steps = task.steps ?? [];
  if (steps.length === 0) return null;
  return Math.round((steps.filter((s) => s.done).length / steps.length) * 100);
}

/** The step being worked on: the first unfinished one. */
export function currentStep(task: Task): Step | null {
  return (task.steps ?? []).find((s) => !s.done) ?? null;
}

/**
 * Whether a task is past its due date.
 *
 * Compared as `YYYY-MM-DD` strings, which sort correctly and avoid a timezone turning "due today"
 * into "overdue" for anyone east of the machine that wrote it.
 */
export function isOverdue(task: Task, today: string): boolean {
  if (!task.due || task.status === "done") return false;
  return task.due < today;
}

/**
 * Due dates as a person reads them, named rather than spelled.
 *
 * A key and its parameters, because this module is pure logic with no locale to render in. It used
 * to return Vietnamese, so an English board said "quá hạn 2026-08-01".
 */
export function dueLabel(due: string, today: string): { key: string; params: { date: string } } {
  const params = { date: due };
  if (due === today) return { key: "tasks.due_today", params };
  if (due < today) return { key: "tasks.due_overdue", params };
  return { key: "tasks.due_on", params };
}
