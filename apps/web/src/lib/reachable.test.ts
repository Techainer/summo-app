import { describe, expect, it } from "vitest";

import { FRESH, delay, saw } from "./reachable";

describe("saw", () => {
  // A daemon restarting after an update drops one request and comes straight back. A bar that
  // flashes on every hiccup is a bar people stop reading, and then it is not there when it matters.
  it("does not cry off on a single dropped request", () => {
    expect(saw(FRESH, false).reachable).toBe(true);
  });

  it("says so once two checks in a row have failed", () => {
    expect(saw(saw(FRESH, false), false).reachable).toBe(false);
  });

  it("keeps saying so while nothing answers", () => {
    let watch = FRESH;
    for (let i = 0; i < 10; i++) watch = saw(watch, false);
    expect(watch.reachable).toBe(false);
  });

  // The daemon coming back is the whole point of continuing to ask.
  it("clears the moment an answer arrives", () => {
    const down = saw(saw(FRESH, false), false);
    expect(saw(down, true)).toEqual(FRESH);
  });

  it("counts consecutive misses, not misses ever", () => {
    const flaky = saw(saw(saw(FRESH, false), true), false);
    expect(flaky.reachable).toBe(true);
  });
});

describe("delay", () => {
  it("looks less often while everything is fine", () => {
    expect(delay(FRESH)).toBeGreaterThan(delay(saw(FRESH, false)));
  });

  // Somebody is now waiting to be let back in, and the retry has to be quick enough that they see
  // the bar go rather than reload the page and lose what is on screen.
  it("asks again within a couple of seconds once something is wrong", () => {
    expect(delay(saw(saw(FRESH, false), false))).toBeLessThanOrEqual(2000);
  });
});
