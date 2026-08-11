import { describe, expect, it } from "vitest";

import { byDay, clock, length, service, titleFrom } from "./notes";

describe("titleFrom", () => {
  // Asking for a title in a separate field before you can type is the step that makes people close
  // a note-taking app.
  it("takes the first line as the title", () => {
    expect(titleFrom("Ý tưởng sản phẩm\n\nGhi nhanh vài dòng.")).toEqual({
      title: "Ý tưởng sản phẩm",
      rest: "Ghi nhanh vài dòng.",
    });
  });

  it("strips a markdown heading marker, because people type one", () => {
    expect(titleFrom("# Họp tuần\nnội dung").title).toBe("Họp tuần");
    expect(titleFrom("### Họp tuần").title).toBe("Họp tuần");
  });

  it("handles a note that is only a title", () => {
    expect(titleFrom("Chỉ một dòng")).toEqual({
      title: "Chỉ một dòng",
      rest: "",
    });
  });

  it("handles an empty note without throwing", () => {
    expect(titleFrom("")).toEqual({ title: "", rest: "" });
  });
});

describe("byDay", () => {
  it("groups and puts the newest day first", () => {
    const grouped = byDay([
      { day: "2026-08-01", id: "old" },
      { day: "2026-08-10", id: "new" },
      { day: "2026-08-01", id: "old2" },
    ]);
    expect(grouped.map(([day]) => day)).toEqual(["2026-08-10", "2026-08-01"]);
    expect(grouped[1]?.[1]).toHaveLength(2);
  });

  it("keeps the order entries arrived in within a day", () => {
    const grouped = byDay([
      { day: "2026-08-10", id: "first" },
      { day: "2026-08-10", id: "second" },
    ]);
    expect(grouped[0]?.[1].map((e) => e.id)).toEqual(["first", "second"]);
  });

  it("handles nothing at all", () => {
    expect(byDay([])).toEqual([]);
  });
});

describe("length", () => {
  it("reads as a person would say it", () => {
    expect(length(1800)).toBe("30m");
    expect(length(3600)).toBe("1h");
    expect(length(5400)).toBe("1h30");
  });

  // The calendar not saying is different from saying zero, and inventing "0m" would be a claim.
  it("renders nothing when the calendar did not say", () => {
    expect(length(null)).toBe("");
    expect(length(0)).toBe("");
  });
});

describe("service", () => {
  it("names the service so a link is recognisable before it is clicked", () => {
    expect(service("https://meet.google.com/abc-defg-hij")).toBe("Meet");
    expect(service("https://zoom.us/j/123")).toBe("Zoom");
    expect(service("https://teams.microsoft.com/l/x")).toBe("Teams");
  });

  it("falls back to a neutral label rather than guessing", () => {
    expect(service("https://example.com/room")).toBe("Link");
  });

  it("says nothing when there is no link", () => {
    expect(service(null)).toBeNull();
  });
});

describe("clock", () => {
  // The daemon encodes a `TZID` local time as if it were UTC, so reading it back as UTC returns
  // the wall-clock time the calendar contains — a 09:00 standup shows as 09:00.
  it("returns the wall-clock time the calendar wrote", () => {
    const nineAm = Date.UTC(2026, 7, 10, 9, 0, 0) / 1000;
    expect(clock(nineAm)).toBe("09:00");
  });

  it("pads both halves so a column of times lines up", () => {
    expect(clock(Date.UTC(2026, 7, 10, 7, 5, 0) / 1000)).toBe("07:05");
    expect(clock(0)).toBe("00:00");
  });
});
