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
 * a table, a footnote, raw HTML, a transcript full of `<!-- seq -->` comments — opens in the plain
 * textarea instead. Falling back is not a failure mode to be minimised away: it is the guarantee.
 * A converter that quietly did its best with a table would eat the table.
 *
 * ## What is supported
 *
 * Headings 1–3, paragraphs, bullet and ordered lists with nesting, task lists (`- [ ] `, which is
 * the same line the task board parses, so ticking a box in a note is ticking it everywhere), fenced
 * code with a language, block quotes, thematic breaks. Inline: bold, italic, inline code, strike,
 * links.
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
    .trim();
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
        /^(-{3,}|\*{3,}|_{3,})\s*$/.test(here)
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

/** The order marks are written in, so nesting is stable across a round trip. */
const ORDER = ["link", "code", "bold", "italic", "strike"];

function text(nodes: Block[] | undefined): string {
  return (nodes ?? [])
    .map((node) => {
      if (node.type === "hardBreak") return "\n";
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
