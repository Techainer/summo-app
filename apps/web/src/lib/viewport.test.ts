import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The one line that makes `env(safe-area-inset-*)` mean anything.
 *
 * Without `viewport-fit=cover` those values are zero on every phone, and the app draws underneath
 * the system bars. On Android that is not subtle: the clock, wifi and battery were painted directly
 * over the app's own title and record button, and the bottom bar's
 * `pb-[env(safe-area-inset-bottom)]` — written when the bar was — had been padding by nothing since
 * the day it was added. Seen for the first time on an emulator with a real phone's resolution; the
 * 320×640 one this project had been using was too small to show it.
 *
 * A test on a meta tag looks like testing HTML. What it is testing is that the four insets used
 * across the interface still resolve to something.
 */
describe("the viewport meta", () => {
  const html = readFileSync(fileURLToPath(new URL("../../index.html", import.meta.url)), "utf8");

  it("asks for the whole screen, insets included", () => {
    expect(html).toMatch(/name="viewport"[^>]*viewport-fit=cover/);
  });

  it("still sets the width and scale a phone needs", () => {
    expect(html).toMatch(/name="viewport"[^>]*width=device-width/);
    expect(html).toMatch(/name="viewport"[^>]*initial-scale=1/);
  });
});
