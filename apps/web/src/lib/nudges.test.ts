import { describe, expect, it } from "vitest";

import { POLL_MS, iconFor } from "./nudges";

describe("iconFor", () => {
  it("gives each reason its own mark so the strip is scannable", () => {
    const icons = (["daily-report", "weekly-rollup", "draft-waiting", "overdue"] as const).map(
      iconFor,
    );
    expect(new Set(icons).size).toBe(icons.length);
  });
});

describe("POLL_MS", () => {
  it("is often enough to be useful and rare enough not to wake a laptop", () => {
    expect(POLL_MS).toBeGreaterThanOrEqual(5 * 60 * 1000);
    expect(POLL_MS).toBeLessThanOrEqual(30 * 60 * 1000);
  });
});
