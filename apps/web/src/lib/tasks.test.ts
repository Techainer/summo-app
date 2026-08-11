import { describe, expect, it } from "vitest";

import { currentStep, dueLabel, forOwner, isOverdue, stepProgress, type Task } from "./tasks";

function task(over: Partial<Task> = {}): Task {
  return {
    id: "T1",
    text: "x",
    status: "todo",
    file: "m.md",
    line: 1,
    ...over,
  };
}

describe("forOwner", () => {
  const tasks = [task({ owner: "ngoc" }), task({ id: "T2", owner: "binh" }), task({ id: "T3" })];

  it("returns everything when no owner is chosen", () => {
    expect(forOwner(tasks, null)).toHaveLength(3);
  });

  it("narrows to one person", () => {
    expect(forOwner(tasks, "ngoc").map((t) => t.id)).toEqual(["T1"]);
  });

  it("does not match an unowned task against a name", () => {
    expect(forOwner(tasks, "khong-ai")).toHaveLength(0);
  });
});

describe("stepProgress", () => {
  it("is null when the agent has not planned yet", () => {
    // Distinct from 0%: no plan is not the same as no progress.
    expect(stepProgress(task())).toBeNull();
    expect(stepProgress(task({ steps: [] }))).toBeNull();
  });

  it("counts finished steps", () => {
    const t = task({
      steps: [
        { text: "a", done: true },
        { text: "b", done: false },
      ],
    });
    expect(stepProgress(t)).toBe(50);
  });

  it("reaches 100 when every step is done", () => {
    expect(stepProgress(task({ steps: [{ text: "a", done: true }] }))).toBe(100);
  });
});

describe("currentStep", () => {
  it("is the first unfinished step", () => {
    const t = task({
      steps: [
        { text: "a", done: true },
        { text: "b", done: false },
        { text: "c", done: false },
      ],
    });
    expect(currentStep(t)?.text).toBe("b");
  });

  it("is null once everything is done", () => {
    expect(currentStep(task({ steps: [{ text: "a", done: true }] }))).toBeNull();
  });

  it("is null when there are no steps", () => {
    expect(currentStep(task())).toBeNull();
  });
});

describe("isOverdue", () => {
  it("is true only once the date has passed", () => {
    expect(isOverdue(task({ due: "2026-08-09" }), "2026-08-10")).toBe(true);
    expect(isOverdue(task({ due: "2026-08-10" }), "2026-08-10")).toBe(false);
    expect(isOverdue(task({ due: "2026-08-11" }), "2026-08-10")).toBe(false);
  });

  it("is false for a finished task, however late", () => {
    // A task done last week is not a problem; nagging about it would be noise.
    expect(isOverdue(task({ due: "2026-01-01", status: "done" }), "2026-08-10")).toBe(false);
  });

  it("is false when nothing is due", () => {
    expect(isOverdue(task(), "2026-08-10")).toBe(false);
  });
});

describe("dueLabel", () => {
  it("says today rather than a date", () => {
    expect(dueLabel("2026-08-10", "2026-08-10").key).toBe("tasks.due_today");
  });

  it("marks a date that has passed", () => {
    expect(dueLabel("2026-08-01", "2026-08-10")).toEqual({
      key: "tasks.due_overdue",
      params: { date: "2026-08-01" },
    });
  });

  it("states a future deadline plainly", () => {
    expect(dueLabel("2026-08-20", "2026-08-10").key).toBe("tasks.due_on");
  });

  /**
   * A key rather than the words. This returned Vietnamese, so an English board read
   * "quá hạn 2026-08-01" — and no catalogue could have fixed it, since the words were never in one.
   */
  it("names what to say rather than saying it", () => {
    for (const due of ["2026-08-01", "2026-08-10", "2026-08-20"]) {
      expect(dueLabel(due, "2026-08-10").key).toMatch(/^tasks\./);
    }
  });
});
