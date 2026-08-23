import { describe, expect, it } from "vitest";

import { ordered } from "./languages";

/**
 * The bug this exists to prevent, in one sentence: a Vietnamese speaker installed Whisper, opened
 * the language list, did not find Vietnamese, and reported that Whisper does not support it.
 *
 * It supports ninety-nine languages including theirs. Sorted by name in Vietnamese, "Tiếng Việt"
 * sorts under T — past seventy-nine other entries and well past the bottom of any dropdown.
 */
describe("ordered", () => {
  it("puts the reader's own language first", () => {
    const codes = ["en", "de", "fr", "vi", "ja"];
    expect(ordered(codes, "vi").map((each) => each.code)[0]).toBe("vi");
    expect(ordered(codes, "ja").map((each) => each.code)[0]).toBe("ja");
    expect(ordered(codes, "en").map((each) => each.code)[0]).toBe("en");
  });

  /// A regional tag is still that language. `vi-VN` is what `navigator.language` reports on a
  /// Vietnamese machine, and matching it strictly against `vi` would sink Vietnamese back down the
  /// list on exactly the machines this is for.
  it("matches a regional locale to its base language", () => {
    expect(ordered(["en", "vi", "ja"], "vi-VN").map((each) => each.code)[0]).toBe("vi");
  });

  /// The four Summo ships in, next, in a fixed order. They are the overwhelmingly likely answers
  /// and a shortlist that reorders itself per locale is a shortlist nobody learns.
  it("follows with the interface languages, minus the one already at the top", () => {
    const order = ordered(["de", "en", "fr", "ja", "vi", "zh"], "vi").map((each) => each.code);
    expect(order.slice(0, 4)).toEqual(["vi", "en", "ja", "zh"]);
  });

  /// And only then the alphabet, in the reader's language rather than in English.
  it("sorts the tail by name in the reader's language", () => {
    const tail = ordered(["de", "fr", "es", "vi"], "vi")
      .map((each) => each.label)
      .slice(1);
    expect(tail).toEqual([...tail].sort((a, b) => a.localeCompare(b, "vi")));
  });

  it("keeps every code it was given, exactly once", () => {
    const codes = ["en", "vi", "ja", "zh", "de", "fr", "es", "ko"];
    const out = ordered(codes, "vi").map((each) => each.code);
    expect([...out].sort()).toEqual([...codes].sort());
  });

  it("survives an empty list and an unknown locale", () => {
    expect(ordered([], "vi")).toEqual([]);
    expect(ordered(["en", "vi"], "xx").map((each) => each.code)).toEqual(["vi", "en"]);
  });
});
