import { describe, expect, it } from "vitest";

import {
  GENTLE,
  METER,
  SNAPPY,
  SPRING,
  collapse,
  listItem,
  screen,
  stagger,
  travel,
} from "./motion";

/**
 * These are constants, so the tests are about the *rules* they have to obey rather than about the
 * numbers themselves. A duration drifting past the point where motion reads as lag is a real
 * regression and an invisible one — nobody notices an interface getting slower by 40 ms at a time.
 */

const duration = (t: unknown): number | null => {
  const value = (t as { duration?: unknown }).duration;
  return typeof value === "number" ? value : null;
};

describe("durations", () => {
  // Above roughly 300 ms an animation stops reading as motion and starts reading as lag.
  it("never runs long enough to feel like waiting", () => {
    for (const [name, transition] of Object.entries({
      SNAPPY,
      GENTLE,
      METER,
    })) {
      const seconds = duration(transition);
      expect(seconds, `${name} has no duration`).not.toBeNull();
      expect(seconds!, `${name} is too slow`).toBeLessThanOrEqual(0.3);
    }
  });

  // Below about 80 ms it may as well not be there, and the change becomes a jump.
  it("never runs so short that nothing is communicated", () => {
    for (const [name, transition] of Object.entries({
      SNAPPY,
      GENTLE,
      METER,
    })) {
      expect(duration(transition)!, `${name} is imperceptible`).toBeGreaterThanOrEqual(0.08);
    }
  });

  // The user has already decided and is waiting for the app to catch up.
  it("moves faster for something the user triggered than for something arriving on its own", () => {
    expect(duration(SNAPPY)!).toBeLessThan(duration(GENTLE)!);
  });
});

describe("the drag spring", () => {
  // Overshoot on a task card landing in a column reads as sloppiness, not as playfulness.
  it("is damped enough not to overshoot a UI control", () => {
    const { stiffness, damping } = SPRING as {
      stiffness: number;
      damping: number;
    };
    // Critical damping is 2·sqrt(k·m); anything at or above it cannot overshoot.
    const mass = (SPRING as { mass: number }).mass;
    expect(damping).toBeGreaterThan(0.9 * 2 * Math.sqrt(stiffness * mass));
  });
});

describe("variants", () => {
  // The failure this catches: a variant renamed in one place and not the other, so an element
  // animates to a state that does not exist and silently stays where it was.
  it("every variant set uses the same three state names", () => {
    for (const [name, set] of Object.entries({ listItem, collapse, screen })) {
      expect(Object.keys(set).sort(), `${name} has different states`).toEqual([
        "gone",
        "hidden",
        "shown",
      ]);
    }
  });

  it("shown is always fully opaque, so nothing settles half-visible", () => {
    for (const [name, set] of Object.entries({ listItem, collapse, screen })) {
      expect((set.shown as { opacity: number }).opacity, `${name}`).toBe(1);
    }
  });

  // Three or four pixels says "this is new". Twenty says the whole list moved.
  it("moves list items and screens a few pixels, not a distance", () => {
    for (const set of [listItem, screen]) {
      for (const state of [set.hidden, set.gone]) {
        const y = (state as { y?: number }).y ?? 0;
        expect(Math.abs(y)).toBeLessThanOrEqual(8);
      }
    }
  });
});

describe("stagger", () => {
  it("assembles a short list with a visible step", () => {
    expect((stagger(4) as { staggerChildren: number }).staggerChildren).toBeGreaterThan(0.02);
  });

  // Past about eight items a constant step becomes a visible wait for the last one.
  it("shortens the step for a long list", () => {
    const short = (stagger(4) as { staggerChildren: number }).staggerChildren;
    const long = (stagger(40) as { staggerChildren: number }).staggerChildren;
    expect(long).toBeLessThan(short);
  });

  it("keeps a long list's total assembly under half a second", () => {
    const step = (stagger(60) as { staggerChildren: number }).staggerChildren;
    expect(step * 60).toBeLessThanOrEqual(1);
  });

  // The rows start at `opacity: 0`, so a step that does not know how long the list is hides the
  // bottom of it. A thousand meetings at the old constant 15 ms put the last row fifteen seconds
  // behind the first, and a vault that size is a year of ordinary use, not a stress test.
  it("holds the whole list to under half a second however long it is", () => {
    for (const total of [200, 1_000, 5_000]) {
      const step = (stagger(total) as { staggerChildren: number }).staggerChildren;
      expect(step * total).toBeLessThanOrEqual(0.5);
    }
  });
});

describe("travel", () => {
  // A long distance is slow to cross and reads as sluggish however short the duration is.
  it("caps how far anything slides, however wide the window", () => {
    expect(travel(2000)).toBe(320);
  });

  it("does not move further than there is room for", () => {
    expect(travel(200)).toBe(200);
  });
});
