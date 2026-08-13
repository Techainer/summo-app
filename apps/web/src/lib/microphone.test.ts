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
  /// The code is what the shell keys the "open permission settings" link off, so a refusal must not
  /// share a code with anything the user cannot fix there.
  it("gives a refused permission its own code", () => {
    const failure = explainMicrophoneError(new DOMException("x", "NotAllowedError"));
    expect(failure.code).toBe("mic_denied");
    expect(failure.error).toMatch(/refused/i);
  });

  it("distinguishes a missing device from one in use by something else", () => {
    expect(explainMicrophoneError(new DOMException("x", "NotFoundError")).code).toBe("mic_missing");
    expect(explainMicrophoneError(new DOMException("x", "NotReadableError")).code).toBe("mic_busy");
  });

  /// Every branch carries English text as well as a code: a locale that has not been updated, or a
  /// browser inventing a `DOMException` name, must still produce a sentence rather than a blank
  /// banner.
  it("falls back to the message rather than saying nothing", () => {
    expect(explainMicrophoneError(new Error("something odd"))).toEqual({
      code: "mic_failed",
      error: "something odd",
    });
    expect(explainMicrophoneError("not an error").error).toMatch(/could not be opened/);
  });
});
