import { describe, expect, it } from "vitest";

import { anchorLabel, inOrder, reacted, segmentOf, writtenAt, type Annotation } from "./comments";

const note = (over: Partial<Annotation> = {}): Annotation => ({
  id: "a1",
  kind: "comment",
  author: "Ngọc",
  at: "2026-08-11T18:00:00+07:00",
  body: "Chốt thứ sáu",
  anchor: { on: "note" },
  resolution: "open",
  ...over,
});

describe("anchorLabel", () => {
  // A badge saying "this is about the note" on every comment in a note is noise.
  it("says nothing for the common case", () => {
    expect(anchorLabel({ on: "note" })).toBeNull();
  });

  it("names the utterance, section or task a comment is pinned to", () => {
    expect(anchorLabel({ on: "segment", seq: 12 })).toBe("#12");
    expect(anchorLabel({ on: "section", heading: "Tóm tắt" })).toBe("Tóm tắt");
    expect(anchorLabel({ on: "task", id: "T4" })).toBe("T4");
  });
});

describe("segmentOf", () => {
  it("reports the utterance so clicking a comment can seek the player", () => {
    expect(segmentOf({ on: "segment", seq: 7 })).toBe(7);
  });

  it("is null for anything not pinned to an utterance", () => {
    expect(segmentOf({ on: "note" })).toBeNull();
    expect(segmentOf({ on: "section", heading: "x" })).toBeNull();
  });

  // Seq 0 is a real utterance — the first one — and `null` must be the only "no".
  it("does not confuse the first utterance with no utterance", () => {
    expect(segmentOf({ on: "segment", seq: 0 })).toBe(0);
  });
});

describe("writtenAt", () => {
  // A comment written at 18:00 in Hanoi should read 18:00 to whoever wrote it. Parsing through
  // `Date` would re-render it in the reader's zone, turning their own words into a different hour.
  it("keeps the hour that was written, whatever zone the reader is in", () => {
    expect(writtenAt("2026-08-11T18:00:00+07:00")).toBe("18:00");
    expect(writtenAt("2026-08-11T18:00:00Z")).toBe("18:00");
  });

  it("renders nothing rather than NaN for a timestamp it cannot read", () => {
    expect(writtenAt("yesterday")).toBe("");
    expect(writtenAt("")).toBe("");
  });
});

describe("inOrder", () => {
  it("sorts by when things were written, not by how they arrived", () => {
    const sorted = inOrder([
      note({ id: "late", at: "2026-08-11T18:00:00+07:00" }),
      note({ id: "early", at: "2026-08-11T09:00:00+07:00" }),
    ]);
    expect(sorted.map((a) => a.id)).toEqual(["early", "late"]);
  });

  it("does not mutate what it was given", () => {
    const input = [note({ id: "b", at: "2026-08-11T18:00:00+07:00" }), note({ id: "a", at: "2026-08-11T09:00:00+07:00" })];
    inOrder(input);
    expect(input[0]?.id).toBe("b");
  });
});

describe("reacted", () => {
  it("knows whether this user already reacted, so the button can toggle", () => {
    const withReaction = note({ reactions: [{ emoji: "👍", by: ["Bình"] }] });
    expect(reacted(withReaction, "👍", "Bình")).toBe(true);
    expect(reacted(withReaction, "👍", "Ngọc")).toBe(false);
    expect(reacted(withReaction, "🎉", "Bình")).toBe(false);
  });

  it("handles a comment nobody has reacted to", () => {
    expect(reacted(note(), "👍", "Bình")).toBe(false);
  });
});
