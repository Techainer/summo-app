import { describe, expect, it } from "vitest";

import { byDay, share, shiftDay, type Report } from "./report";

/** A report with only the fields `byDay` reads, so a change elsewhere does not rewrite this file. */
function report(from: string, to: string, meetings: [string, number][]): Report {
  return {
    from,
    to,
    meetings: meetings.map(([day, duration], i) => ({
      id: String(i),
      title: `m${i}`,
      day,
      duration,
      participants: [],
      tags: [],
      has_summary: false,
    })),
    total_seconds: meetings.reduce((sum, [, duration]) => sum + duration, 0),
    people: [],
    tags: [],
    open_actions: [],
    done_actions: 0,
    without_summary: [],
    quiet_days: [],
  };
}

describe("byDay", () => {
  it("covers the whole window, including the days nothing happened on", () => {
    const strip = byDay(report("2026-08-17", "2026-08-19", [["2026-08-18", 600]]));
    expect(strip.map((d) => d.day)).toEqual(["2026-08-17", "2026-08-18", "2026-08-19"]);
    expect(strip.map((d) => d.seconds)).toEqual([0, 600, 0]);
  });

  it("adds up several meetings on one day", () => {
    const strip = byDay(
      report("2026-08-18", "2026-08-18", [
        ["2026-08-18", 600],
        ["2026-08-18", 900],
      ]),
    );
    expect(strip).toEqual([{ day: "2026-08-18", seconds: 1500, count: 2 }]);
  });

  it("crosses a month boundary", () => {
    const strip = byDay(report("2026-07-30", "2026-08-02", []));
    expect(strip.map((d) => d.day)).toEqual([
      "2026-07-30",
      "2026-07-31",
      "2026-08-01",
      "2026-08-02",
    ]);
  });

  // A window whose end is before its start, or whose dates are not dates at all, must not spin: the
  // loop walks forward and would never meet its end.
  it("stops on a window it cannot walk", () => {
    expect(byDay(report("2026-08-19", "2026-08-17", []))).toHaveLength(31);
    expect(byDay(report("not-a-day", "2026-08-17", []))).toHaveLength(1);
  });
});

describe("share", () => {
  it("is a percentage, and zero rather than NaN when nothing was recorded", () => {
    expect(share(30, 120)).toBe(25);
    expect(share(0, 0)).toBe(0);
  });
});

describe("shiftDay", () => {
  it("moves whole days in UTC, so a timezone cannot move the boundary", () => {
    expect(shiftDay("2026-08-31", 1)).toBe("2026-09-01");
    expect(shiftDay("2026-01-01", -1)).toBe("2025-12-31");
  });
});
