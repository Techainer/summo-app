import { describe, expect, it } from "vitest";

import { bytes } from "./storage";

describe("bytes", () => {
  it("says small numbers plainly", () => {
    expect(bytes(0, "vi")).toBe("0 B");
    expect(bytes(512, "vi")).toBe("512 B");
  });

  it("steps up a unit at a time", () => {
    expect(bytes(2048, "vi")).toBe("2 KB");
    expect(bytes(5 * 1024 * 1024, "vi")).toBe("5 MB");
    expect(bytes(3 * 1024 ** 3, "vi")).toBe("3 GB");
  });

  // A comma, in the language that writes decimals with one. The number beside a disk is the last
  // place an app should switch to somebody else's punctuation.
  it("writes the decimal mark the reader's language uses", () => {
    expect(bytes(1.5 * 1024 * 1024, "vi")).toBe("1,5 MB");
    expect(bytes(1.5 * 1024 * 1024, "en")).toBe("1.5 MB");
  });

  // Two hundred megabytes does not need a decimal; one and a half does.
  it("drops the fraction once the number is big enough to read", () => {
    expect(bytes(200 * 1024 * 1024, "en")).toBe("200 MB");
  });
});
