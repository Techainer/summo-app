import { describe, expect, it } from "vitest";
import { accepts, encodeFrame, formatTime, isTranscript, type Event } from "./protocol";

describe("segment merge policy", () => {
  it("lets a final replace a partial", () => {
    expect(accepts("partial", "final")).toBe(true);
  });

  it("refuses a late partial over a final", () => {
    // A partial re-decode can land after the final it belongs to; applying it would make committed
    // text flicker back to an earlier guess.
    expect(accepts("final", "partial")).toBe(false);
  });

  it("lets the refine model revise a final", () => {
    expect(accepts("final", "revised")).toBe(true);
  });

  it("never overwrites a hand edit", () => {
    for (const next of ["partial", "final", "revised"] as const) {
      expect(accepts("manual", next)).toBe(false);
    }
  });

  it("matches the Rust policy on every pair", () => {
    // Divergence here shows up as text that flickers or reverts, which is hard to trace back.
    const table: Array<[string, string, boolean]> = [
      ["partial", "partial", true],
      ["partial", "revised", true],
      ["final", "final", true],
      ["revised", "revised", true],
      ["revised", "partial", false],
      ["revised", "final", false],
    ];
    for (const [current, next, expected] of table) {
      expect(accepts(current as never, next as never)).toBe(expected);
    }
  });
});

describe("frame encoding", () => {
  it("writes the lane tag then little-endian samples", () => {
    const buffer = encodeFrame("system", new Float32Array([1, -1]));
    const view = new DataView(buffer);

    expect(buffer.byteLength).toBe(9);
    expect(view.getUint8(0)).toBe(1);
    expect(view.getFloat32(1, true)).toBe(1);
    expect(view.getFloat32(5, true)).toBe(-1);
  });

  it("tags the microphone lane as zero", () => {
    expect(new DataView(encodeFrame("mic", new Float32Array([0]))).getUint8(0)).toBe(0);
  });
});

describe("event narrowing", () => {
  it("recognises the three transcript kinds", () => {
    const partial = { kind: "partial", seq: 1, lane: "mic", text: "x", t0: 0, t1: 1, source: "partial" } as Event;
    expect(isTranscript(partial)).toBe(true);
    expect(isTranscript({ kind: "info", text: "hi" })).toBe(false);
    expect(isTranscript({ kind: "stat", rtf: 0.1, rss_mb: 10, queue_ms: 0 })).toBe(false);
  });
});

describe("time formatting", () => {
  it("uses MM:SS under an hour and H:MM:SS over it", () => {
    expect(formatTime(0)).toBe("00:00");
    expect(formatTime(65)).toBe("01:05");
    expect(formatTime(3725)).toBe("1:02:05");
  });

  it("does not render negative time", () => {
    expect(formatTime(-5)).toBe("00:00");
  });
});
