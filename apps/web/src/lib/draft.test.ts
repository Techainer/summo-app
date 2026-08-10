import { describe, expect, it } from "vitest";

import { draftHeadings, isRefinable, readable, type Draft } from "./draft";

function draft(headings: string[]): Draft {
  return {
    meeting: "01A",
    template: "standard",
    sections: headings.map((heading) => ({ heading, body: "x" })),
    turns: [],
    revisions: 0,
  };
}

describe("draftHeadings", () => {
  it("is empty when nothing is waiting", () => {
    expect(draftHeadings(null).size).toBe(0);
    expect(draftHeadings(draft([])).size).toBe(0);
  });

  it("lists the sections still marked as the agent's", () => {
    const headings = draftHeadings(draft(["Tóm tắt", "Quyết định"]));
    expect(headings.has("Tóm tắt")).toBe(true);
    expect(headings.has("Ghi chú của tôi")).toBe(false);
  });
});

describe("isRefinable", () => {
  it("rejects an accidental double-click", () => {
    // One or two words is almost never a deliberate "rewrite this".
    expect(isRefinable("ngân")).toBe(false);
    expect(isRefinable("ngân sách")).toBe(false);
  });

  it("accepts a real passage", () => {
    expect(isRefinable("chốt ngân sách quý bốn")).toBe(true);
  });

  it("ignores surrounding whitespace", () => {
    expect(isRefinable("   một hai ba   ")).toBe(true);
  });
});

describe("readable", () => {
  it("hides the machine-readable state the vault carries in comments", () => {
    const body = "- [ ] @ngoc Chốt spec <!-- id:T1 status:doing -->";
    expect(readable(body)).toBe("- [ ] @ngoc Chốt spec");
  });

  it("leaves ordinary prose alone", () => {
    expect(readable("Chốt ngân sách quý bốn.")).toBe("Chốt ngân sách quý bốn.");
  });

  it("handles a comment spanning lines", () => {
    expect(readable("một <!--\nhai\n--> ba")).toBe("một  ba");
  });

  it("keeps line structure so a list still reads as a list", () => {
    expect(readable("- một <!-- x -->\n- hai <!-- y -->")).toBe("- một\n- hai");
  });
});
