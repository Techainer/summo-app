import { describe, expect, it } from "vitest";

import { PAUSE_SECONDS, decorate, italicise } from "./reading";
import type { Segment } from "./protocol";

function segment(over: Partial<Segment> & { t0: number; t1: number }): Segment {
  return { seq: 1, lane: "system", text: "…", source: "final", ...over };
}

describe("grouping", () => {
  it("names the first speaker", () => {
    const [row] = decorate([segment({ t0: 0, t1: 2, speaker: "Ngọc" })]);
    expect(row?.showSpeaker).toBe(true);
  });

  // A name above every line turns a paragraph into a list and doubles its height. On a phone that
  // is the difference between seeing the last thirty seconds and the last ten.
  it("does not repeat a speaker who is still talking", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, speaker: "Ngọc" }),
      segment({ t0: 2, t1: 4, speaker: "Ngọc" }),
      segment({ t0: 4, t1: 6, speaker: "Ngọc" }),
    ]);
    expect(rows.map((r) => r.showSpeaker)).toEqual([true, false, false]);
  });

  it("names a new speaker", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, speaker: "Ngọc" }),
      segment({ t0: 2, t1: 4, speaker: "Dũng" }),
    ]);
    expect(rows[1]?.showSpeaker).toBe(true);
  });

  // By the time a long silence has passed, the name has scrolled away and "who is this" is the
  // first question again.
  it("names the same speaker again after a long silence", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, speaker: "Ngọc" }),
      segment({ t0: 2 + PAUSE_SECONDS, t1: 30, speaker: "Ngọc" }),
    ]);
    expect(rows[1]?.showSpeaker).toBe(true);
    expect(rows[1]?.pause).toBeGreaterThanOrEqual(PAUSE_SECONDS);
  });

  // Two unnamed voices are not one unnamed voice: the microphone is whoever is holding it and the
  // system lane is whoever is on the call.
  it("does not group two unnamed voices from different lanes", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, lane: "mic" }),
      segment({ t0: 2, t1: 4, lane: "system" }),
    ]);
    expect(rows[1]?.showSpeaker).toBe(true);
  });
});

describe("people talking over each other", () => {
  // The failure this exists for: a single microphone hears two people at once, and a plain list
  // renders them one after the other — which reads as a reply. It is not a reply.
  it("marks an utterance that began before the last one ended", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 5, speaker: "Ngọc" }),
      segment({ t0: 3, t1: 7, speaker: "Dũng" }),
    ]);
    expect(rows[1]?.overlapping).toBe(true);
  });

  it("does not mark a clean handover", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 5, speaker: "Ngọc" }),
      segment({ t0: 5, t1: 7, speaker: "Dũng" }),
    ]);
    expect(rows[1]?.overlapping).toBe(false);
  });

  // One person's utterances often overlap by a fraction of a second where the recogniser split
  // them. That is one person talking, not two, and drawing it as simultaneous speech would be
  // wrong on almost every line.
  it("does not mark one speaker overlapping themselves", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 5, speaker: "Ngọc" }),
      segment({ t0: 4.8, t1: 9, speaker: "Ngọc" }),
    ]);
    expect(rows[1]?.overlapping).toBe(false);
  });

  it("does not also report a pause for an overlap", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 5, speaker: "Ngọc" }),
      segment({ t0: 3, t1: 7, speaker: "Dũng" }),
    ]);
    expect(rows[1]?.pause).toBeNull();
  });
});

describe("pauses", () => {
  it("reports a long silence", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, speaker: "Ngọc" }),
      segment({ t0: 12, t1: 14, speaker: "Ngọc" }),
    ]);
    expect(rows[1]?.pause).toBe(10);
  });

  // Below a few seconds a gap is a breath, or the recogniser closing an utterance early. Marking
  // those would put a break in the middle of a sentence.
  it("ignores a breath", () => {
    const rows = decorate([
      segment({ t0: 0, t1: 2, speaker: "Ngọc" }),
      segment({ t0: 3, t1: 5, speaker: "Ngọc" }),
    ]);
    expect(rows[1]?.pause).toBeNull();
  });
});

describe("italics", () => {
  // CJK has no italics, so a browser shears the glyphs instead. On dense characters that is harder
  // to read and looks like a rendering fault.
  it("are not used for scripts that do not have them", () => {
    for (const language of ["zh", "ja", "ko", "th", "ar", "he", "zh-Hant", "JA"]) {
      expect(italicise(language), language).toBe(false);
    }
  });

  it("are used where they mean something", () => {
    for (const language of ["en", "vi", "fr", "de", "en-GB"]) {
      expect(italicise(language), language).toBe(true);
    }
  });
});
