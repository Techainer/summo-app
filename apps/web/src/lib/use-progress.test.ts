import { describe, expect, it } from "vitest";

import { perSecond, roughly } from "./use-progress";

describe("perSecond", () => {
  it("scales the unit to the number", () => {
    expect(perSecond(512)).toBe("512 B/s");
    expect(perSecond(210_000)).toBe("210 KB/s");
    expect(perSecond(4_200_000)).toBe("4.2 MB/s");
  });

  /// Nothing rather than "0 B/s": an unknown rate and a stalled one are different, and the caller
  /// filters empty strings out of the line instead of printing a zero that looks like a failure.
  it("says nothing when there is nothing to say", () => {
    expect(perSecond(null)).toBe("");
    expect(perSecond(0)).toBe("");
    expect(perSecond(-1)).toBe("");
  });
});

describe("roughly", () => {
  /// Under ninety seconds, to the nearest five. A countdown ticking 47, 46, 45 is a countdown
  /// somebody watches instead of leaving, and the estimate is not accurate to the second anyway.
  it("rounds seconds to five", () => {
    expect(roughly(43)).toEqual({ unit: "sec", value: 45 });
    expect(roughly(2)).toEqual({ unit: "sec", value: 5 });
  });

  /// The case that started all this: SMALL100 at the rate it was measured downloading.
  it("reports a fifty-minute download in minutes", () => {
    expect(roughly(2_940)).toEqual({ unit: "min", value: 49 });
  });

  it("switches to hours past ninety minutes", () => {
    expect(roughly(9_000)).toEqual({ unit: "hour", value: 2.5 });
  });

  /// A rate of zero divides to `Infinity`, which must not render as "about Infinity minutes left".
  it("refuses to estimate what it cannot", () => {
    expect(roughly(null)).toBeNull();
    expect(roughly(0)).toBeNull();
    expect(roughly(Number.POSITIVE_INFINITY)).toBeNull();
    expect(roughly(Number.NaN)).toBeNull();
  });

  /// Never zero. "About 0 minutes left" on a download that is still running reads as a stuck app.
  it("never says zero", () => {
    expect(roughly(1)?.value).toBeGreaterThan(0);
    expect(roughly(95)?.value).toBeGreaterThan(0);
  });
});
