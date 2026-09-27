import { describe, expect, it } from "vitest";
import {
  dayLabel,
  groupLabel,
  localDay,
  ordered,
  playable,
  swatch,
  timeOfDay,
  timestamp,
  url,
  type MeetingSummary,
} from "./library";

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
    const built = url(handshake, "/library", {
      group: "day",
      folder: "",
      without_summary: false,
    });
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

describe("swatch", () => {
  it("turns a palette name into the theme's variable for it", () => {
    expect(swatch("teal")).toBe("var(--color-swatch-teal)");
  });

  it("is nothing when a document has no colour", () => {
    expect(swatch(null)).toBeUndefined();
    expect(swatch(undefined)).toBeUndefined();
    expect(swatch("")).toBeUndefined();
  });

  /**
   * The reason this function exists. A colour comes out of a file the user edits by hand and ends
   * up in a `style` attribute, so the one place that conversion happens is the one place it has to
   * be impossible to escape the `var(` it sits inside.
   */
  it("refuses anything that could close the var() it sits in", () => {
    for (const attack of [
      "teal)",
      "teal); background: url(https://evil.example/p.png",
      "red; --color-bg: red",
      "#0f7350",
      "url(x)",
      "TEAL",
      "swatch-teal", // a hyphen is not a letter, and this is how a prefix would be smuggled in
    ]) {
      expect(swatch(attack), attack).toBeUndefined();
    }
  });

  /**
   * The daemon owns the palette and sends it; this only checks the shape. A name that shapes up
   * but has no token resolves to nothing, which is a missing dot rather than a broken screen — so
   * a colour added to the daemon before the theme still degrades quietly.
   */
  it("accepts a name the theme may not have a token for yet", () => {
    expect(swatch("indigo")).toBe("var(--color-swatch-indigo)");
  });
});

/** Enough of a summary for the comparisons under test. */
function row(over: Partial<MeetingSummary>): MeetingSummary {
  return {
    kind: "meeting",
    id: "x",
    title: "",
    folder: "",
    parent: null,
    date: "2026-01-01T09:00:00+07:00",
    day: "2026-01-01",
    duration: 0,
    participants: [],
    tags: [],
    color: null,
    has_summary: false,
    size_bytes: 0,
    file: "a.md",
    ...over,
  };
}

describe("ordering the library", () => {
  const march = row({ id: "m", date: "2026-03-01T09:00:00+07:00", title: "Đầu", duration: 60 });
  const june = row({ id: "j", date: "2026-06-01T09:00:00+07:00", title: "Cuối", duration: 600 });
  const groups = [
    { key: "2026-06-01", meetings: [june] },
    { key: "2026-03-01", meetings: [march] },
  ];

  it("puts the newest first by default", () => {
    const out = ordered([{ key: "k", meetings: [march, june] }], "recent", "vi");
    expect(out[0]?.meetings.map((m) => m.id)).toEqual(["j", "m"]);
  });

  it("turns the whole view around for oldest first, headings included", () => {
    // A day heading is part of the order. Leaving June above March while the meetings inside each
    // ran upwards would be two directions on one screen.
    const out = ordered(groups, "oldest", "vi");
    expect(out.map((g) => g.key)).toEqual(["2026-03-01", "2026-06-01"]);
  });

  it("sorts titles the way the reader's language does", () => {
    const d = row({ id: "d", title: "Duyệt" });
    const dd = row({ id: "dd", title: "Đánh giá" });
    const e = row({ id: "e", title: "Export" });
    const out = ordered([{ key: "k", meetings: [e, dd, d] }], "title", "vi");
    // In Vietnamese `Đ` sorts after `D` and before `E`; by code unit it would land after `Z`.
    expect(out[0]?.meetings.map((m) => m.id)).toEqual(["d", "dd", "e"]);
  });

  it("sorts by length without touching the dates", () => {
    const out = ordered([{ key: "k", meetings: [march, june] }], "longest", "vi");
    expect(out[0]?.meetings.map((m) => m.id)).toEqual(["j", "m"]);
  });
});

describe("what a meeting can play", () => {
  it("offers the lanes the audio route serves", () => {
    expect(playable(["mic.opus", "system.opus"])).toEqual(["mic", "system"]);
  });

  it("ignores files that are not lanes", () => {
    // An imported meeting used to draw a transport whose only lane answered `no such lane`.
    expect(playable(["summary.json", "import.wav"])).toEqual(["import"]);
    expect(playable(["notes.md"])).toEqual([]);
  });
});
