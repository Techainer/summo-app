import { describe, expect, it } from "vitest";
import { apply, edit, empty, renameSpeaker } from "./transcript";
import type { Event } from "./protocol";

const seg = (kind: "partial" | "final" | "revise", seq: number, text: string): Event => ({
  kind,
  seq,
  lane: "mic",
  text,
  t0: 0,
  t1: 1,
  source: "partial",
});

describe("transcript store", () => {
  it("adds a new segment", () => {
    const state = apply(empty(), seg("partial", 1, "xin"));
    expect(state.segments).toHaveLength(1);
    expect(state.segments[0]?.text).toBe("xin");
  });

  it("replaces a partial in place rather than appending", () => {
    let state = apply(empty(), seg("partial", 1, "xin"));
    state = apply(state, seg("partial", 1, "xin chào"));
    expect(state.segments).toHaveLength(1);
    expect(state.segments[0]?.text).toBe("xin chào");
  });

  it("ignores a late partial after the final", () => {
    let state = apply(empty(), seg("final", 1, "xin chào"));
    const before = state;
    state = apply(state, seg("partial", 1, "xin ch"));
    // Identity, not equality: an unchanged state must be the same object so React can skip a render.
    expect(state).toBe(before);
    expect(state.segments[0]?.text).toBe("xin chào");
  });

  it("lets a revision replace a final", () => {
    let state = apply(empty(), seg("final", 1, "toi nghi"));
    state = apply(state, seg("revise", 1, "Tôi nghĩ"));
    expect(state.segments[0]?.text).toBe("Tôi nghĩ");
    expect(state.segments[0]?.source).toBe("revised");
  });

  it("keeps segments in arrival order", () => {
    let state = empty();
    for (const s of [1, 2, 3]) state = apply(state, seg("final", s, `câu ${s}`));
    expect(state.segments.map((s) => s.seq)).toEqual([1, 2, 3]);
  });

  it("never lets a model overwrite a hand edit", () => {
    let state = apply(empty(), seg("final", 1, "sai chính tả"));
    state = edit(state, 1, "đã sửa tay");
    state = apply(state, seg("revise", 1, "bản model nghĩ là đúng"));
    expect(state.segments[0]?.text).toBe("đã sửa tay");
  });

  it("keeps a speaker label through a revision that omits one", () => {
    // The refine model does not do diarization, so its output carries no speaker. Applying it must
    // not blank the label the clusterer already assigned.
    let state = apply(empty(), {
      ...(seg("final", 1, "câu") as object),
      speaker: "S2",
    } as Event);
    state = apply(state, seg("revise", 1, "câu đã sửa"));
    expect(state.segments[0]?.speaker).toBe("S2");
  });

  it("renames a speaker everywhere at once", () => {
    let state = empty();
    state = apply(state, {
      ...(seg("final", 1, "a") as object),
      speaker: "S1",
    } as Event);
    state = apply(state, {
      ...(seg("final", 2, "b") as object),
      speaker: "S2",
    } as Event);
    state = apply(state, {
      ...(seg("final", 3, "c") as object),
      speaker: "S1",
    } as Event);

    state = renameSpeaker(state, "S1", "Ngọc");
    expect(state.segments.map((s) => s.speaker)).toEqual(["Ngọc", "S2", "Ngọc"]);
  });

  it("ignores non-transcript events", () => {
    const state = empty();
    expect(apply(state, { kind: "info", text: "hi" })).toBe(state);
    expect(apply(state, { kind: "stat", rtf: 0.1, rss_mb: 1, queue_ms: 0 })).toBe(state);
  });

  it("editing an unknown segment is a no-op", () => {
    const state = empty();
    expect(edit(state, 99, "x")).toBe(state);
  });
});

describe("live translation", () => {
  const withLine = () => apply(empty(), seg("final", 1, "xin chào"));

  it("attaches to the line it names without replacing the original", () => {
    const state = apply(withLine(), {
      kind: "translation",
      seq: 1,
      lang: "en",
      text: "hello",
    });

    expect(state.segments[0]?.text).toBe("xin chào");
    expect(state.segments[0]?.translation).toEqual({
      lang: "en",
      text: "hello",
    });
  });

  // Out-of-order delivery would otherwise invent a segment with no text, no speaker and no timing,
  // which renders as a blank line in the middle of the transcript.
  it("drops a translation for a line that has not arrived", () => {
    const before = empty();
    const after = apply(before, {
      kind: "translation",
      seq: 99,
      lang: "en",
      text: "hello",
    });
    expect(after).toBe(before);
  });

  it("is replaced when a second translation arrives for the same line", () => {
    let state = apply(withLine(), {
      kind: "translation",
      seq: 1,
      lang: "en",
      text: "hi",
    });
    state = apply(state, {
      kind: "translation",
      seq: 1,
      lang: "en",
      text: "hello there",
    });
    expect(state.segments[0]?.translation?.text).toBe("hello there");
  });

  it("survives a revision of the line underneath it", () => {
    let state = apply(withLine(), {
      kind: "translation",
      seq: 1,
      lang: "en",
      text: "hello",
    });
    state = apply(state, seg("revise", 1, "xin chào các bạn"));

    expect(state.segments[0]?.text).toBe("xin chào các bạn");
    expect(state.segments[0]?.translation?.text).toBe("hello");
  });
});
