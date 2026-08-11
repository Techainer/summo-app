import { describe, expect, it } from "vitest";
import { formatDuration } from "./duration";

describe("formatDuration", () => {
  it("says hours and minutes the way a person would", () => {
    expect(formatDuration(2538, "vi-VN")).toBe("42 phút");
    expect(formatDuration(3600, "vi-VN")).toBe("1 giờ");
    expect(formatDuration(5400, "vi-VN")).toBe("1 giờ 30 phút");
  });

  /**
   * The reason this module exists. Three copies of it were spread across `library.ts`, `people.ts`
   * and `report.ts`, all of them saying `phút` whatever language the interface was in — so the
   * English build read "1 giờ 27 phút" on its dashboard.
   */
  it("is in the reader's language, not the author's", () => {
    expect(formatDuration(2538, "en-US")).toBe("42 minutes");
    expect(formatDuration(5400, "en-US")).toBe("1 hour 30 minutes");
  });

  /** English pluralises and Vietnamese does not; `Intl` knows that and a catalogue would have to. */
  it("gets plurals right without being told the rule", () => {
    expect(formatDuration(60, "en-US")).toBe("1 minute");
    expect(formatDuration(120, "en-US")).toBe("2 minutes");
    expect(formatDuration(7200, "en-US")).toBe("2 hours");
  });

  /**
   * The three copies disagreed here: one rounded a twenty-second note up to "1 phút", one said
   * "0 phút", one said "20 giây". The last is the only one that answers the question — the
   * difference between a twenty-second note and a one-minute one is the whole of what was asked.
   */
  it("keeps seconds rather than rounding a short recording into a minute", () => {
    expect(formatDuration(20, "vi-VN")).toBe("20 giây");
    expect(formatDuration(59, "en-US")).toBe("59 seconds");
  });

  /** No duration means it was typed, not recorded. A zero invites the reader to suspect a bug. */
  it("shows nothing rather than zero for something that was never recorded", () => {
    expect(formatDuration(0, "vi-VN")).toBe("—");
    expect(formatDuration(-1, "en-US")).toBe("—");
  });
});

describe("the short form", () => {
  /**
   * A dashboard tile is a small box with a headline number in it. "1 hour 27 minutes" wrapped onto
   * two lines and grew the tile past its neighbours; "1 hr 27 min" is the same fact in a third of
   * the width, and `Intl` knows the abbreviation for each locale.
   */
  it("fits a number into a tile without changing what it says", () => {
    expect(formatDuration(5220, "en-US", "short")).toBe("1 hr 27 min");
    expect(formatDuration(5220, "en-US")).toBe("1 hour 27 minutes");
  });

  it("still shows nothing for something that was never recorded", () => {
    expect(formatDuration(0, "en-US", "short")).toBe("—");
  });
});
