import { describe, expect, it } from "vitest";

import { initials } from "./avatar";

describe("initials", () => {
  it("takes the last two words, because Vietnamese given names come last", () => {
    // Taking the first two would label a very large number of people "NT".
    expect(initials("Nguyễn Thị Ngọc")).toBe("TN");
    expect(initials("Trần Văn Bình")).toBe("VB");
  });

  it("keeps a single word to one letter", () => {
    expect(initials("Ngọc")).toBe("N");
  });

  it("handles names outside the Latin script", () => {
    // `[...word][0]` rather than `word[0]`: a surrogate pair sliced in half renders as a replacement
    // character, and the CJK and emoji cases are the ones that would hit it.
    expect(initials("田中 太郎")).toBe("田太");
    expect(initials("张伟")).toBe("张");
  });

  it("does not throw on a name that is only spaces", () => {
    expect(initials("   ")).toBe("?");
    expect(initials("")).toBe("?");
  });
});
