import { describe, expect, it } from "vitest";
import { dayLabel, formatDuration, groupLabel, localDay, timeOfDay, timestamp, url } from "./library";

/** Vietnamese, because that is what these assertions are written against. */
const VI = {
  locale: "vi-VN",
  today: "Hôm nay",
  yesterday: "Hôm qua",
  week: "Tuần {n}, {year}",
  unfiled: "Chưa phân loại",
};

describe("dayLabel", () => {
  const today = "2026-08-10";

  it("names the days a person still remembers", () => {
    expect(dayLabel("2026-08-10", today, VI)).toBe("Hôm nay");
    expect(dayLabel("2026-08-09", today, VI)).toBe("Hôm qua");
    // `Intl`'s own Vietnamese, capitalisation included. It is the authority on how a weekday is
    // written in a locale; the hand-written table this replaced was one language's guess.
    expect(dayLabel("2026-08-06", today, VI)).toBe("Thứ Năm");
  });

  it("falls back to a date once the weekday stops meaning anything", () => {
    expect(dayLabel("2026-07-01", today, VI)).toBe("1 tháng 7");
    expect(dayLabel("2025-12-24", today, VI)).toBe("24 tháng 12, 2025");
  });

  it("does not shift a day into the browser's timezone", () => {
    // Parsed as local time, `2026-08-10` would be a different calendar day west of UTC, and the
    // heading would say "Hôm qua" to a user in Los Angeles for a meeting they had this morning.
    expect(dayLabel("2026-08-10", "2026-08-10", VI)).toBe("Hôm nay");
  });

  it("passes through anything that is not a date", () => {
    expect(dayLabel("", today, VI)).toBe("");
    expect(dayLabel("not-a-day", today, VI)).toBe("not-a-day");
  });
});

describe("groupLabel", () => {
  it("reads an ISO week as a week", () => {
    expect(groupLabel("2026-W32", "week", "2026-08-10", VI)).toBe("Tuần 32, 2026");
    expect(groupLabel("2026-W02", "week", "2026-08-10", VI)).toBe("Tuần 2, 2026");
  });

  it("names the folder a meeting has not been filed into", () => {
    expect(groupLabel("", "folder", "2026-08-10", VI)).toBe("Chưa phân loại");
    expect(groupLabel("khach-hang/acme", "folder", "2026-08-10", VI)).toBe("khach-hang/acme");
  });
});

describe("formatDuration", () => {
  it("reads as time rather than as a count of seconds", () => {
    expect(formatDuration(0)).toBe("—");
    expect(formatDuration(2538)).toBe("42 phút");
    expect(formatDuration(3600)).toBe("1 giờ");
    expect(formatDuration(5400)).toBe("1 giờ 30 phút");
  });

  it("never rounds a real recording down to nothing", () => {
    // A 20-second note is not a zero-minute meeting.
    expect(formatDuration(20)).toBe("1 phút");
  });
});

describe("timeOfDay", () => {
  it("takes the clock time from the meeting's own offset", () => {
    expect(timeOfDay("2026-08-09T23:30:00+07:00")).toBe("23:30");
  });
});

describe("timestamp", () => {
  it("grows an hours field only when it needs one", () => {
    expect(timestamp(724)).toBe("12:04");
    expect(timestamp(3725)).toBe("1:02:05");
    expect(timestamp(-1)).toBe("0:00");
  });
});

describe("url", () => {
  const handshake = { port: 8710, token: "secret" };

  it("carries the token and drops empty filters", () => {
    const built = url(handshake, "/library", { group: "day", folder: "", without_summary: false });
    expect(built).toBe("http://127.0.0.1:8710/library?token=secret&group=day");
  });

  it("escapes what a user typed", () => {
    expect(url(handshake, "/library/search", { q: "họp & ngân sách" })).toContain(
      "q=h%E1%BB%8Dp+%26+ng%C3%A2n+s%C3%A1ch",
    );
  });

  it("omits the token when the app was not given one", () => {
    expect(url({ port: 8710, token: "" }, "/library")).toBe("http://127.0.0.1:8710/library");
  });
});

describe("localDay", () => {
  it("is the browser's calendar day, not a UTC one", () => {
    expect(localDay(new Date(2026, 7, 10, 1, 0))).toBe("2026-08-10");
    expect(localDay(new Date(2026, 0, 1))).toBe("2026-01-01");
  });
});
