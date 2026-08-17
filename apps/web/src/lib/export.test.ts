import { describe, expect, it } from "vitest";

import { fileName } from "./export";

describe("the name a download arrives under", () => {
  it("is the day and the title, in characters every filesystem takes", () => {
    expect(fileName("Họp đầu tuần", "2026-08-16", "srt")).toBe("2026-08-16-hop-dau-tuan.srt");
  });

  it("says which language it is, when it is not the one that was spoken", () => {
    expect(fileName("Demo khách hàng", "2026-08-09", "vtt", "en")).toBe(
      "2026-08-09-demo-khach-hang-en.vtt",
    );
  });

  // `đ` has no combining form, so stripping diacritics leaves it behind and a Windows share
  // rejects the rest anyway. It becomes `d`, which is what every Vietnamese slug does.
  it("handles đ, which is not an accent", () => {
    expect(fileName("Đặt hàng", "2026-01-02", "txt")).toBe("2026-01-02-dat-hang.txt");
  });

  it("still produces a name when the title is only punctuation", () => {
    expect(fileName("???", "2026-01-02", "md")).toBe("2026-01-02-summo.md");
  });
});
