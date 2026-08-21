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
    ["a table", "| Tên | Việc |\n| --- | --- |\n| Ngọc | API |\n| Vinh | UI |"],
    ["a table with one column", "| Tên |\n| --- |\n| Ngọc |"],
    ["a table with no body rows", "| Tên | Việc |\n| --- | --- |"],
    [
      "a table with every alignment",
      "| trái | giữa | phải | không |\n| :--- | :---: | ---: | --- |\n| a | b | c | d |",
    ],
    ["marks inside a cell", "| Ai |\n| --- |\n| **Ngọc** và `api` |"],
    ["a link inside a cell", "| Ở đâu |\n| --- |\n| [tài liệu](https://summo.app) |"],
    ["an escaped pipe inside a cell", "| Lệnh |\n| --- |\n| a \\| b |"],
    ["a break inside a cell", "| Ghi |\n| --- |\n| một<br>hai |"],
    ["a table under a heading", "## Bảng\n| a |\n| --- |\n| 1 |"],
    ["a table between paragraphs", "Trên.\n\n| a |\n| --- |\n| 1 |\n\nDưới."],
    ["an image on its own line", "![sơ đồ](attachments/aaaa.png)"],
    ["an image with no alt text", "![](attachments/aaaa.png)"],
    ["an image inside a sentence", "xem ![sơ đồ](attachments/aaaa.png) nhé"],
    ["an image and a link together", "![a](attachments/a.png) và [b](https://summo.app)"],
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
    ["a transcript line", "**[00:00:01] Ngọc** — xin chào <!-- seq:0 end:2.00 -->"],
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

describe("tables", () => {
  /**
   * Two rows of pipes with no divider between them are two paragraphs in every Markdown
   * implementation there is. Rendering them as a table would mean this editor showed something no
   * other tool does — and then wrote a divider into the user's file to make itself right.
   */
  it("needs a divider row, not just pipes", () => {
    const doc = toDoc("| a | b |\n| 1 | 2 |");
    expect(doc.content?.every((node) => node.type === "paragraph")).toBe(true);
    expect(round("| a | b |\n| 1 | 2 |")).toBe("| a | b |\n| 1 | 2 |");
  });

  /**
   * Almost every table anything else has touched has its pipes lined up into a grid. Treating that
   * as a difference would send every one of those notes to the plain textarea.
   */
  it("does not mind how the pipes were lined up", () => {
    expect(faithful("| Tên  | Việc |\n| ---- | ---- |\n| Ngọc | API  |")).toBe(true);
    expect(faithful("|a|b|\n|-|-|\n|1|2|")).toBe(true);
  });

  it("puts the column's alignment on its cells, which is where ProseMirror keeps it", () => {
    const table = toDoc("| a | b |\n| :---: | ---: |\n| 1 | 2 |").content?.[0];
    const heads = table?.content?.[0]?.content;
    expect(heads?.map((cell) => cell.attrs?.align as string)).toEqual(["center", "right"]);
    // And on the body row too, so a cell always knows how it is drawn.
    expect(table?.content?.[1]?.content?.[0]?.attrs?.align).toBe("center");
  });

  it("builds header cells for the first row and ordinary cells below it", () => {
    const table = toDoc("| a |\n| --- |\n| 1 |").content?.[0];
    expect(table?.content?.[0]?.content?.[0]?.type).toBe("tableHeader");
    expect(table?.content?.[1]?.content?.[0]?.type).toBe("tableCell");
  });

  /**
   * An unescaped pipe in a cell is a new column — corruption that looks like a typo. The editor
   * cannot stop somebody typing one, so the serializer has to write it back escaped.
   */
  it("escapes a pipe somebody typed into a cell", () => {
    const doc = toDoc("| a |\n| --- |\n| x |");
    const cell = doc.content?.[0]?.content?.[1]?.content?.[0];
    cell!.content = [{ type: "paragraph", content: [{ type: "text", text: "a | b" }] }];
    expect(toMarkdown(doc)).toContain("| a \\| b |");
  });

  /**
   * A cell holds one line in the file and more than one on screen. `<br>` is where the difference
   * goes: it is what every Markdown renderer already shows as a break inside a cell, so the note
   * still reads correctly in Obsidian.
   */
  it("writes a second paragraph in a cell as a break rather than losing it", () => {
    const doc = toDoc("| a |\n| --- |\n| một |");
    const cell = doc.content?.[0]?.content?.[1]?.content?.[0];
    cell!.content = [
      { type: "paragraph", content: [{ type: "text", text: "một" }] },
      { type: "paragraph", content: [{ type: "text", text: "hai" }] },
    ];
    const written = toMarkdown(doc);
    expect(written).toContain("| một<br>hai |");
    // And it comes back as the break it became, so the file is stable from here on.
    expect(round(written.trim())).toBe(written.trim());
  });

  /**
   * A body row with fewer cells than the header comes back exactly as written — this converter does
   * not square it up. Which is the point of the *second* check: ProseMirror will pad that row when
   * it loads the document, and `RichNote` compares what the schema produced against the file, so
   * the note opens as text and keeps its ragged table. Text-level fidelity is necessary here and
   * not sufficient, and this is the case that shows the difference.
   */
  it("reproduces a ragged table rather than squaring it up", () => {
    const ragged = "| a | b |\n| --- | --- |\n| 1 |";
    expect(round(ragged)).toBe(ragged);
    expect(toDoc(ragged).content?.[0]?.content?.[1]?.content?.length).toBe(1);
  });
});

describe("images", () => {
  it("reads a picture as a node rather than as the characters that spell it", () => {
    const paragraph = toDoc("![sơ đồ](attachments/aa.png)").content?.[0];
    const image = paragraph?.content?.[0];
    expect(image?.type).toBe("image");
    expect(image?.attrs?.src).toBe("attachments/aa.png");
    expect(image?.attrs?.alt).toBe("sơ đồ");
  });

  /** `![alt](src)` contains `[alt](src)`; matching the link first leaves a stray `!` behind. */
  it("is not mistaken for a link with an exclamation mark in front", () => {
    const paragraph = toDoc("![a](x.png)").content?.[0];
    expect(paragraph?.content?.length).toBe(1);
    expect(paragraph?.content?.[0]?.type).toBe("image");
  });
});

describe("highlight", () => {
  it("survives a round trip", () => {
    const doc = toDoc("Chốt **thứ năm**, ==ngân sách 200tr==.");
    expect(toMarkdown(doc).trim()).toBe("Chốt **thứ năm**, ==ngân sách 200tr==.");
  });

  it("is a mark, not a paragraph of equals signs", () => {
    const doc = toDoc("==xong==");
    const paragraph = doc.content?.[0];
    const marks = paragraph?.content?.[0]?.marks?.map((m) => m.type);
    expect(marks).toEqual(["highlight"]);
  });
});
