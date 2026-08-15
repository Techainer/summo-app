import { describe, expect, it } from "vitest";

import { faithful, toDoc, toMarkdown } from "./markdown";

/**
 * The converter behind the rich note editor, and the guard that decides whether to use it.
 *
 * These tests carry more weight than most in this repository. The vault is Markdown a person opens
 * in Obsidian and backs up, and the failure this code can cause is *silent*: a note that lost its
 * table on save still looks like a note. So the interesting assertions are not "the editor renders
 * a heading" — they are "text goes in and comes out identical", and "when it does not, `faithful`
 * says so and the rich editor is never offered".
 */

const round = (markdown: string) => toMarkdown(toDoc(markdown)).trim();

describe("round trip", () => {
  const survives = [
    ["a paragraph", "Ghi nhanh vài dòng."],
    ["two paragraphs", "Một.\n\nHai."],
    ["a soft break inside one paragraph", "Một dòng\nvà dòng nữa"],
    ["headings", "# Một\n## Hai\n### Ba"],
    ["a bullet list", "- một\n- hai"],
    ["a numbered list", "1. một\n2. hai"],
    ["a numbered list that does not start at one", "3. ba\n4. bốn"],
    ["a nested bullet list", "- một\n  - lồng\n  - nữa\n- hai"],
    ["a to-do list", "- [ ] chưa xong\n- [x] xong rồi"],
    ["a to-do line the task board parses", "- [ ] @ngoc Chốt spec API"],
    ["a block quote", "> trích\n> dẫn"],
    ["a thematic break", "trên\n\n---\n\ndưới"],
    ["fenced code", "```rust\nfn main() {}\n```"],
    ["fenced code with no language", "```\nplain\n```"],
    ["a heading inside a fence stays code", "```\n## không phải tiêu đề\n```"],
    ["bold", "**đậm** và thường"],
    ["italic", "*nghiêng* và thường"],
    ["inline code", "gõ `cargo test` đi"],
    ["strikethrough", "~~bỏ~~ rồi"],
    ["a link", "xem [tài liệu](https://summo.app) nhé"],
    ["bold inside a link", "[**tài liệu**](https://summo.app)"],
    ["a list under a heading", "## Việc\n- [ ] một\n- [x] hai"],
    ["prose, a list and a quote together", "Mở đầu.\n\n- một\n- hai\n\n> kết"],
  ] as const;

  for (const [what, markdown] of survives) {
    it(`keeps ${what}`, () => {
      expect(round(markdown)).toBe(markdown);
      expect(faithful(markdown)).toBe(true);
    });
  }
});

describe("what it does not model", () => {
  /**
   * Anything with no node for it stays literal text, and that is the safe answer rather than a
   * lucky one: the round trip still reproduces the file byte for byte, so a table nobody can
   * format in the editor is at least a table nobody can lose in it. Structure this converter
   * invents is what would eat the file.
   */
  const literal = [
    ["a table", "| a | b |\n|---|---|\n| 1 | 2 |"],
    ["a transcript line", "**[00:00:01] Ngọc** — xin chào <!-- seq:0 end:2.00 -->"],
    ["an image", "![ảnh](a.png)"],
    ["raw HTML", "<div>gì đó</div>"],
    ["a heading deeper than three", "#### bốn"],
    ["a footnote", "câu[^1]\n\n[^1]: chú thích"],
  ] as const;

  for (const [what, markdown] of literal) {
    it(`keeps ${what} as text rather than reshaping it`, () => {
      expect(round(markdown)).toBe(markdown);
      expect(toDoc(markdown).content?.every((node) => node.type === "paragraph")).toBe(true);
    });
  }

  it("is false rather than throwing on anything it cannot read", () => {
    expect(faithful("```\nunterminated")).toBe(false);
  });

  it("ignores whitespace that carries nothing", () => {
    expect(faithful("một   \n\n\n\nhai")).toBe(true);
  });

  /**
   * The vault writes a section's body on the line under its heading — `trim_section` trims both
   * ends — while a person typing in Obsidian leaves a blank line. Treating those as different is
   * what sent every note to the plain textarea the first time this was wired up.
   */
  it("treats a blank line after a heading as the same document", () => {
    expect(faithful("## Việc\n\nlàm gì đó")).toBe(true);
    expect(toMarkdown(toDoc("## Việc\n\nlàm gì đó")).trim()).toBe("## Việc\nlàm gì đó");
  });
});

describe("the document it builds", () => {
  it("marks a ticked box as checked, because that is what the file says", () => {
    const doc = toDoc("- [x] xong");
    const list = doc.content?.[0];
    expect(list?.type).toBe("taskList");
    expect(list?.content?.[0]?.attrs?.checked).toBe(true);
  });

  it("keeps the language on a fence, so the file does not lose it", () => {
    expect(toDoc("```ts\nx\n```").content?.[0]?.attrs?.language).toBe("ts");
  });

  it("nests a list inside the item it was indented under", () => {
    const doc = toDoc("- một\n  - lồng");
    const item = doc.content?.[0]?.content?.[0];
    expect(item?.content?.[1]?.type).toBe("bulletList");
  });

  it("does not join a bullet run to a to-do run", () => {
    // Writing them back as one list would put checkboxes on lines that never had them.
    const doc = toDoc("- một\n- [ ] hai");
    expect(doc.content?.map((node) => node.type)).toEqual(["bulletList", "taskList"]);
  });

  it("reads an empty document as an empty document rather than a blank paragraph", () => {
    expect(toDoc("").content).toEqual([]);
  });
});
