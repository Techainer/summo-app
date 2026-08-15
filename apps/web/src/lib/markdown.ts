import type { JSONContent } from "@tiptap/react";

/**
 * Markdown in, Markdown out — and a way to know when that is not true.
 *
 * The vault is Markdown a person opens in Obsidian, greps and backs up. A rich editor that stores
 * its own shape breaks that promise the first time it round-trips something it does not understand,
 * and it breaks it *silently*: the file still looks like a note. So this is deliberately a small,
 * readable converter over the shapes the vault actually uses, rather than a general Markdown
 * implementation — and it is paired with [`faithful`], which is what makes it safe to use at all.
 *
 * ## The rule
 *
 * The editor is offered only for a note that survives a round trip **unchanged**. Anything else —
 * a footnote, raw HTML, a transcript full of `<!-- seq -->` comments — opens in the plain textarea
 * instead. Falling back is not a failure mode to be minimised away: it is the guarantee. A
 * converter that quietly did its best with a footnote would eat the footnote.
 *
 * ## What is supported
 *
 * Headings 1–3, paragraphs, bullet and ordered lists with nesting, task lists (`- [ ] `, which is
 * the same line the task board parses, so ticking a box in a note is ticking it everywhere), fenced
 * code with a language, block quotes, thematic breaks, GFM tables with column alignment. Inline:
 * bold, italic, inline code, strike, links, images.
 */

/** Two spaces per level, which is what the serializer emits and what the parser counts in. */
const INDENT = 2;

interface Block {
  type: string;
  attrs?: Record<string, unknown>;
  content?: Block[];
  text?: string;
  marks?: { type: string; attrs?: Record<string, unknown> }[];
}

/**
 * Whitespace differences that carry no information, removed from both sides of a comparison.
 *
 * The blank line after a heading is one of them, and it is the reason this function exists rather
 * than a plain `===`. The vault's own serializer writes `## Heading` and the body on the next line
 * with nothing between — `trim_section` in `summo_vault::meeting` trims both ends of a section — so
 * a note that came off disk and a note this converter wrote would otherwise differ by one newline
 * and every note would fall back to the plain textarea.
 */
function tidy(markdown: string): string {
  return markdown
    .replace(/[ \t]+$/gm, "")
    .replace(/\n{3,}/g, "\n\n")
    .replace(/^(#{1,3} .*)\n\n/gm, "$1\n")
    .split("\n")
    .map(evenly)
    .join("\n")
    .trim();
}

/**
 * A table row with its columns' padding removed.
 *
 * Almost every table written by hand — and every table any other editor has touched — has its pipes
 * lined up into a grid. That padding is a picture of the table rather than part of it, and without
 * this every such note would fail the round trip and open in the plain textarea, which is the
 * opposite of the point.
 *
 * The consequence is worth stating plainly: once a person *edits* a note with a padded table, it is
 * written back unpadded. The grid is not preserved. That is the cost of the editor managing the
 * table at all, and it is paid only on a note somebody changed — opening one and leaving it alone
 * writes nothing, because [`same`] compares through this.
 */
function evenly(line: string): string {
  if (!/^\s*\|.*\|\s*$/.test(line)) return line;
  const values = cells(line);
  const rule = values.every((value) => DIVIDER.test(value));
  return `| ${values.map((value) => (rule ? divider(alignOf(value)) : value)).join(" | ")} |`;
}

/**
 * Whether two pieces of Markdown say the same thing.
 *
 * Textual identity once trailing spaces and runs of blank lines are gone, because those carry no
 * information and the vault's own serializer emits a different number of them than this one does —
 * a document of empty headings comes off disk as `## A\n\n\n## B`. Anything else is a difference,
 * including one this converter thinks is an improvement.
 */
export function same(a: string, b: string): boolean {
  return tidy(a) === tidy(b);
}

/**
 * Whether this note can be edited richly without losing anything.
 *
 * The whole reason to have this check is that the failure it guards against is invisible — a note
 * that lost its table still looks like a note — so the bar is that writing back what was read
 * reproduces the file.
 */
export function faithful(markdown: string): boolean {
  try {
    return same(toMarkdown(toDoc(markdown)), markdown);
  } catch {
    return false;
  }
}

/** Markdown to a ProseMirror document. */
export function toDoc(markdown: string): JSONContent {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  return { type: "doc", content: blocks(lines, 0) };
}

/**
 * Read a run of lines into blocks, at one level of list indentation.
 *
 * `depth` is how many spaces of indentation belong to the enclosing list, so a nested list is the
 * same function called again rather than a second code path that drifts from the first.
 */
function blocks(lines: string[], depth: number): Block[] {
  const out: Block[] = [];
  let at = 0;

  const strip = (line: string) => line.slice(depth);
  const indent = (line: string) => line.length - line.trimStart().length;

  while (at < lines.length) {
    const raw = lines[at] ?? "";
    if (raw.trim() === "") {
      at += 1;
      continue;
    }
    const line = strip(raw);

    // Fenced code first, so a `## ` inside a fence is code and not a heading.
    const fence = /^```(\S*)\s*$/.exec(line);
    if (fence) {
      const body: string[] = [];
      at += 1;
      while (at < lines.length && !/^```\s*$/.test(strip(lines[at] ?? ""))) {
        body.push(strip(lines[at] ?? ""));
        at += 1;
      }
      at += 1; // the closing fence
      out.push({
        type: "codeBlock",
        attrs: { language: fence[1] || null },
        content: body.length > 0 ? [{ type: "text", text: body.join("\n") }] : undefined,
      });
      continue;
    }

    const heading = /^(#{1,3}) (.*)$/.exec(line);
    if (heading) {
      out.push({
        type: "heading",
        attrs: { level: heading[1]!.length },
        content: inline(heading[2]!),
      });
      at += 1;
      continue;
    }

    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      out.push({ type: "horizontalRule" });
      at += 1;
      continue;
    }

    // Before the paragraph fallback and after the rule, because `| --- |` is a table divider and
    // `---` on its own is a thematic break.
    const table = tableAt(lines, at, strip);
    if (table) {
      out.push(table.node);
      at = table.at;
      continue;
    }

    if (line.startsWith("> ") || line === ">") {
      const quoted: string[] = [];
      while (at < lines.length) {
        const next = strip(lines[at] ?? "");
        if (!next.startsWith(">")) break;
        quoted.push(next.replace(/^>\s?/, ""));
        at += 1;
      }
      out.push({ type: "blockquote", content: blocks(quoted, 0) });
      continue;
    }

    const kind = kindOf(line);
    if (kind !== null) {
      // A run of items at this indentation, each item taking any deeper-indented lines with it.
      const task = kind === "taskList";
      const items: Block[] = [];
      const start = Number(/^(\d+)\. /.exec(line)?.[1] ?? 1);

      while (at < lines.length) {
        const next = lines[at] ?? "";
        if (next.trim() === "") {
          // A blank line ends the list unless the following line is still inside it.
          const after = lines[at + 1] ?? "";
          if (after.trim() === "" || indent(after) < depth) break;
          at += 1;
          continue;
        }
        if (indent(next) !== depth) break;
        const here = strip(next);
        // A run is one kind. A bullet list that turns into a to-do list is two lists, and writing
        // them back as one would put checkboxes on lines that never had them.
        if (kindOf(here) !== kind) break;

        let text = here.replace(/^([-*]|\d+\.) /, "");
        let checked = false;
        if (task) {
          checked = /^\[[xX]\] /.test(text);
          text = text.replace(/^\[[ xX]\] /, "");
        }
        at += 1;

        // Everything indented further belongs to this item.
        const nested: string[] = [];
        while (at < lines.length) {
          const child = lines[at] ?? "";
          if (child.trim() !== "" && indent(child) <= depth) break;
          nested.push(child);
          at += 1;
        }
        while (nested.length > 0 && (nested.at(-1) ?? "").trim() === "") nested.pop();

        const content: Block[] = [{ type: "paragraph", content: inline(text) }];
        if (nested.length > 0) content.push(...blocks(nested, depth + INDENT));
        items.push(
          task ? { type: "taskItem", attrs: { checked }, content } : { type: "listItem", content },
        );
      }

      out.push({
        type: kind,
        ...(kind === "orderedList" ? { attrs: { start } } : {}),
        content: items,
      });
      continue;
    }

    // A paragraph: this line and the lines under it until something else starts.
    const para: string[] = [];
    while (at < lines.length) {
      const next = lines[at] ?? "";
      if (next.trim() === "") break;
      const here = strip(next);
      if (
        /^#{1,3} /.test(here) ||
        /^```/.test(here) ||
        /^[-*] /.test(here) ||
        /^\d+\. /.test(here) ||
        here.startsWith(">") ||
        /^(-{3,}|\*{3,}|_{3,})\s*$/.test(here) ||
        tableAt(lines, at, strip) !== null
      ) {
        if (para.length > 0) break;
      }
      para.push(here);
      at += 1;
    }
    out.push({ type: "paragraph", content: inline(para.join("\n")) });
  }

  return out;
}

/** A divider cell: `---`, `:---`, `---:` or `:---:`. One dash is enough, per GFM. */
const DIVIDER = /^:?-+:?$/;

/** Which way a column is aligned, from its divider cell. `null` is "however it renders". */
function alignOf(cell: string): Align {
  const start = cell.startsWith(":");
  const end = cell.endsWith(":");
  if (start && end) return "center";
  if (end) return "right";
  if (start) return "left";
  return null;
}

/** The divider cell an alignment is written as. */
function divider(align: Align): string {
  if (align === "center") return ":---:";
  if (align === "right") return "---:";
  if (align === "left") return ":---";
  return "---";
}

type Align = "left" | "center" | "right" | null;

/**
 * One row split into cells, `\|` surviving as a literal pipe.
 *
 * The leading and trailing pipes are dropped rather than producing empty cells at each end, which
 * is what a naive `split("|")` does and is the reason a table written the ordinary way came out
 * with a blank first column.
 */
function cells(line: string): string[] {
  const body = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  const out: string[] = [];
  let current = "";
  for (let at = 0; at < body.length; at += 1) {
    if (body[at] === "\\" && body[at + 1] === "|") {
      current += "|";
      at += 1;
      continue;
    }
    if (body[at] === "|") {
      out.push(current.trim());
      current = "";
      continue;
    }
    current += body[at];
  }
  out.push(current.trim());
  return out;
}

/**
 * A table starting at this line, or `null` if one does not.
 *
 * The divider row is what makes a table a table: two rows of pipes with nothing between them are
 * two paragraphs in every Markdown implementation there is, and treating them as a table here would
 * mean this editor rendered something no other tool does.
 *
 * A body row with a different number of cells than the header is taken as written. ProseMirror will
 * square it up when the document is loaded, the round-trip check will see that it did, and the note
 * opens as text with its ragged table intact — which is the right outcome for a file this editor
 * cannot hold without changing it.
 */
function tableAt(
  lines: string[],
  from: number,
  strip: (line: string) => string,
): { node: Block; at: number } | null {
  const header = strip(lines[from] ?? "").trim();
  const rule = strip(lines[from + 1] ?? "").trim();
  if (!header.startsWith("|") || !rule.startsWith("|")) return null;

  const heads = cells(header);
  const dividers = cells(rule);
  if (dividers.length !== heads.length || !dividers.every((cell) => DIVIDER.test(cell)))
    return null;

  const align = dividers.map(alignOf);
  const rows: Block[] = [row(heads, align, true)];
  let at = from + 2;
  while (at < lines.length) {
    const line = strip(lines[at] ?? "").trim();
    if (!line.startsWith("|")) break;
    rows.push(row(cells(line), align, false));
    at += 1;
  }
  return { node: { type: "table", content: rows }, at };
}

/**
 * One row of cells.
 *
 * The alignment is written onto every cell rather than held once for the column, because that is
 * where ProseMirror keeps it — a table has no column objects, only rows of cells. The serializer
 * reads it back off the header row, which is the only row GFM can express it from.
 */
function row(values: string[], align: Align[], head: boolean): Block {
  return {
    type: "tableRow",
    content: values.map((value, at) => ({
      type: head ? "tableHeader" : "tableCell",
      attrs: { colspan: 1, rowspan: 1, colwidth: null, align: align[at] ?? null },
      // `<br>` because a cell holds one line in GFM and more than one on screen. It is what every
      // Markdown renderer already shows as a break inside a cell, and it is the only way a person
      // who pressed Enter in a cell keeps what they typed.
      content: [{ type: "paragraph", content: inline(value.split(BREAK).join("\n")) }],
    })),
  };
}

/** `<br>`, however it was written. */
const BREAK = /<br\s*\/?>/i;

/** Which kind of list a line starts, or `null` if it starts none. */
function kindOf(line: string): "bulletList" | "orderedList" | "taskList" | null {
  if (/^[-*] \[[ xX]\] /.test(line)) return "taskList";
  if (/^[-*] /.test(line)) return "bulletList";
  if (/^\d+\. /.test(line)) return "orderedList";
  return null;
}

/** The inline marks, innermost last, so `**a *b***` nests the way the serializer writes it. */
const INLINE: { mark: string; pattern: RegExp }[] = [
  { mark: "code", pattern: /`([^`]+)`/ },
  { mark: "bold", pattern: /\*\*([^*]+)\*\*/ },
  { mark: "italic", pattern: /(?<!\*)\*([^*\n]+)\*(?!\*)/ },
  { mark: "strike", pattern: /~~([^~]+)~~/ },
];

/**
 * Text with marks on it.
 *
 * A hard break rather than a paragraph per line: two lines of one paragraph in the file are one
 * paragraph on screen, and splitting them would put a blank line into the file that the user never
 * typed.
 */
function inline(text: string): Block[] {
  const pieces = text.split("\n");
  const out: Block[] = [];
  pieces.forEach((piece, at) => {
    if (at > 0) out.push({ type: "hardBreak" });
    out.push(...marked(piece));
  });
  return out.filter((node) => node.type !== "text" || (node.text ?? "") !== "");
}

function marked(text: string): Block[] {
  if (text === "") return [];

  // Before links, because `![alt](src)` contains `[alt](src)` — matching the link first would leave
  // a stray `!` in the text and an image nobody could see.
  const picture = /!\[([^\]]*)\]\(([^)\s]+)\)/.exec(text);
  if (picture) {
    const [whole, alt, src] = picture;
    return [
      ...marked(text.slice(0, picture.index)),
      { type: "image", attrs: { src: src!, alt: alt || null, title: null } },
      ...marked(text.slice(picture.index + whole.length)),
    ];
  }

  // A link is handled first because its label can carry marks of its own.
  const link = /\[([^\]]+)\]\(([^)\s]+)\)/.exec(text);
  if (link) {
    const [whole, label, href] = link;
    const at = link.index;
    return [
      ...marked(text.slice(0, at)),
      ...marked(label!).map((node) => ({
        ...node,
        marks: [...(node.marks ?? []), { type: "link", attrs: { href } }],
      })),
      ...marked(text.slice(at + whole.length)),
    ];
  }

  for (const { mark, pattern } of INLINE) {
    const found = pattern.exec(text);
    if (!found) continue;
    const [whole, body] = found;
    const at = found.index;
    const inner =
      mark === "code"
        ? [{ type: "text", text: body!, marks: [{ type: "code" }] }]
        : marked(body!).map((node) => ({
            ...node,
            marks: [...(node.marks ?? []), { type: mark }],
          }));
    return [...marked(text.slice(0, at)), ...inner, ...marked(text.slice(at + whole.length))];
  }

  return [{ type: "text", text }];
}

/**
 * A ProseMirror document back to Markdown, in the form the vault writes.
 *
 * A blank line between blocks, except after a heading — `summo_vault::meeting` puts a section's
 * body on the line directly under its heading, and writing a different shape here would mean the
 * file changed the moment somebody opened it.
 */
export function toMarkdown(doc: JSONContent): string {
  const nodes = (doc.content ?? []) as Block[];
  let out = "";
  nodes.forEach((node, at) => {
    const text = render(node, 0);
    if (text === "") return;
    if (at > 0) out += nodes[at - 1]?.type === "heading" ? "\n" : "\n\n";
    out += text;
  });
  return `${out}\n`;
}

function render(node: Block, depth: number): string {
  const pad = " ".repeat(depth);
  switch (node.type) {
    case "heading":
      return `${pad}${"#".repeat(Number(node.attrs?.level ?? 1))} ${text(node.content)}`;
    case "paragraph":
      return `${pad}${text(node.content)}`;
    case "horizontalRule":
      return `${pad}---`;
    case "codeBlock": {
      const language = (node.attrs?.language as string | null) ?? "";
      const body = (node.content ?? []).map((child) => child.text ?? "").join("");
      return `${pad}\`\`\`${language}\n${body}\n${pad}\`\`\``;
    }
    case "blockquote":
      return (node.content ?? [])
        .map((child) => render(child, 0))
        .join("\n\n")
        .split("\n")
        .map((line) => `${pad}> ${line}`.trimEnd())
        .join("\n");
    case "table": {
      const rows = node.content ?? [];
      const head = rows[0];
      if (!head) return "";
      // Off the header row, because a column's alignment is one fact and GFM writes it once. Every
      // cell carries it — see [`row`] — and any of them would do; the header is the row that is
      // always there.
      const align = (head.content ?? []).map((cell) => (cell.attrs?.align as Align) ?? null);
      const line = (r: Block) =>
        `${pad}| ${(r.content ?? []).map((cell) => inCell(cell)).join(" | ")} |`;
      return [
        line(head),
        `${pad}| ${align.map(divider).join(" | ")} |`,
        ...rows.slice(1).map(line),
      ].join("\n");
    }
    case "bulletList":
    case "orderedList":
    case "taskList": {
      const start = Number(node.attrs?.start ?? 1);
      return (node.content ?? [])
        .map((child, at) => {
          const marker =
            node.type === "orderedList"
              ? `${start + at}.`
              : node.type === "taskList"
                ? `- [${child.attrs?.checked ? "x" : " "}]`
                : "-";
          return item(marker, child, depth);
        })
        .join("\n");
    }
    default:
      return `${pad}${text(node.content)}`;
  }
}

/** One list item: its first paragraph on the marker's line, anything else indented under it. */
function item(marker: string, node: Block, depth: number): string {
  const pad = " ".repeat(depth);
  const [first, ...rest] = node.content ?? [];
  const head = `${pad}${marker} ${first ? text(first.content) : ""}`.trimEnd();
  if (rest.length === 0) return head;
  return [head, ...rest.map((child) => render(child, depth + INDENT))].join("\n");
}

/**
 * What one cell says, on one line.
 *
 * A cell holds paragraphs on screen and a single line in the file, and the difference has to go
 * somewhere. `<br>` is where: it is what every Markdown renderer already shows as a break inside a
 * cell, so the file reads correctly in Obsidian and comes back here as the break it was.
 *
 * A pipe is escaped, because an unescaped one would silently become a new column — the kind of
 * corruption that looks like a typo and is not.
 */
function inCell(cell: Block): string {
  return (cell.content ?? [])
    .map((block) => text(block.content))
    .join("<br>")
    .replace(/\|/g, "\\|")
    .split("\n")
    .join("<br>")
    .trim();
}

/** The order marks are written in, so nesting is stable across a round trip. */
const ORDER = ["link", "code", "bold", "italic", "strike"];

function text(nodes: Block[] | undefined): string {
  return (nodes ?? [])
    .map((node) => {
      if (node.type === "hardBreak") return "\n";
      if (node.type === "image") {
        const src = node.attrs?.src;
        const alt = node.attrs?.alt;
        return `![${typeof alt === "string" ? alt : ""}](${typeof src === "string" ? src : ""})`;
      }
      let out = node.text ?? "";
      const marks = [...(node.marks ?? [])].sort(
        (a, b) => ORDER.indexOf(b.type) - ORDER.indexOf(a.type),
      );
      for (const mark of marks) {
        if (mark.type === "bold") out = `**${out}**`;
        else if (mark.type === "italic") out = `*${out}*`;
        else if (mark.type === "code") out = `\`${out}\``;
        else if (mark.type === "strike") out = `~~${out}~~`;
        else if (mark.type === "link") {
          const href = mark.attrs?.href;
          out = `[${out}](${typeof href === "string" ? href : ""})`;
        }
      }
      return out;
    })
    .join("");
}
