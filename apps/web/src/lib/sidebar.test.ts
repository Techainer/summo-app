// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";

import { STORAGE_KEY, remember, wasOpen } from "./sidebar";

describe("the sidebar's remembered state", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("starts open, because that is the app with its navigation on", () => {
    expect(wasOpen()).toBe(true);
  });

  it("remembers a collapse across a reload", () => {
    remember(false);
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("closed");
    expect(wasOpen()).toBe(false);
  });

  it("remembers opening it again", () => {
    remember(false);
    remember(true);
    expect(wasOpen()).toBe(true);
  });

  // Anything else in that key is somebody else's data or a half-written value, and the app it
  // belongs to should still start with its navigation showing.
  it("treats a value it did not write as open", () => {
    window.localStorage.setItem(STORAGE_KEY, "wat");
    expect(wasOpen()).toBe(true);
  });
});
