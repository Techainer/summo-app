import { describe, expect, it } from "vitest";
import { explainMicrophoneError, rms } from "./microphone";

describe("signal level", () => {
  it("is zero for silence and one for full scale", () => {
    expect(rms(new Float32Array(64))).toBe(0);
    expect(rms(new Float32Array(0))).toBe(0);

    const square = Float32Array.from({ length: 64 }, (_, i) => (i % 2 === 0 ? 1 : -1));
    expect(rms(square)).toBeCloseTo(1, 5);
  });
});

describe("microphone errors", () => {
  it("sends a refused permission to settings, not to the hardware", () => {
    const message = explainMicrophoneError(new DOMException("x", "NotAllowedError"));
    expect(message).toMatch(/refused/i);
    expect(message).toMatch(/settings/i);
  });

  it("distinguishes a missing device from one in use by something else", () => {
    expect(explainMicrophoneError(new DOMException("x", "NotFoundError"))).toMatch(/No microphone/);
    expect(explainMicrophoneError(new DOMException("x", "NotReadableError"))).toMatch(
      /another application/,
    );
  });

  it("falls back to the message rather than saying nothing", () => {
    expect(explainMicrophoneError(new Error("something odd"))).toBe("something odd");
    expect(explainMicrophoneError("not an error")).toMatch(/could not be opened/);
  });
});
