import { describe, expect, it } from "vitest";

import { sourceOf, visible } from "./source";

describe("what a reader can see", () => {
  it("drops the markers and keeps the words", () => {
    expect(visible("Chốt **ngân sách** quý bốn.").text).toBe("Chốt ngân sách quý bốn.");
    expect(visible("- Thiếu người").text).toBe("Thiếu người");
    expect(visible("- [ ] @Ngọc Chốt spec").text).toBe("@Ngọc Chốt spec");
    expect(visible("### Rủi ro").text).toBe("Rủi ro");
  });

  it("shows a link's label and not its address", () => {
    expect(visible("Xem [bản spec](https://x.invalid/a) nhé").text).toBe("Xem bản spec nhé");
  });

  it("drops the state the vault keeps in comments", () => {
    expect(visible("- [ ] Gọi khách <!-- id:01T1 status:todo -->").text).toBe("Gọi khách ");
  });

  it("keeps a nested item's indentation, because that is space on the screen", () => {
    expect(visible("- Chậm hàng\n  - Nhà cung cấp B").text).toBe("Chậm hàng\n  Nhà cung cấp B");
  });

  it("maps every visible character back to where it came from", () => {
    const source = "- **Chốt** ngân sách";
    const seen = visible(source);
    for (let at = 0; at < seen.text.length; at += 1) {
      expect(source[seen.map[at]!]).toBe(seen.text[at]);
    }
  });
});

describe("finding the source of a selection", () => {
  it("returns the Markdown behind what was selected", () => {
    const body = "Chốt **ngân sách** quý bốn.";
    // What `window.getSelection().toString()` gives for a drag across the bold phrase and after.
    expect(sourceOf(body, "ngân sách quý bốn")).toBe("**ngân sách** quý bốn");
  });

  it("cuts a task line out with its marker left alone", () => {
    const body = "- [ ] @Ngọc Chốt spec API <!-- id:01T1 -->";
    expect(sourceOf(body, "Chốt spec API")).toBe("Chốt spec API");
  });

  // Half a `**` left in the file is a section that renders as nonsense from then on.
  it("takes the emphasis it started inside with it", () => {
    expect(sourceOf("Chốt **ngân sách** quý bốn.", "ngân sách")).toBe("**ngân sách**");
    expect(sourceOf("Một `mã lệnh` ở giữa", "mã lệnh")).toBe("`mã lệnh`");
  });

  it("gives up rather than guess when the phrase appears twice", () => {
    expect(sourceOf("Gọi khách. Gọi khách.", "Gọi khách")).toBeNull();
  });

  it("gives up when the phrase is not in this section at all", () => {
    expect(sourceOf("Chốt ngân sách.", "gửi báo giá")).toBeNull();
  });

  // The daemon splices on a verbatim match, so whatever comes back has to be in the file.
  it("returns something the file actually contains", () => {
    const body = "## Việc\n- [ ] Xem [bản spec](https://x.invalid) trước <!-- id:A -->\n";
    const cut = sourceOf(body, "Xem bản spec trước");
    expect(cut).not.toBeNull();
    expect(body).toContain(cut);
  });
});
