import type { JSONContent } from "@tiptap/react";
import { type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { toDoc } from "../../lib/markdown";

/**
 * A section of a page, drawn as what it says rather than as how it is stored.
 *
 * Every section of every meeting used to be one `<p>` of the file's own bytes with
 * `whitespace-pre-wrap` on it. On the screen people read after a meeting — the one screen this app
 * exists to produce — a list of decisions appeared as `- decided X`, the actions as
 * `- [ ] @Ngọc Chốt spec API`, a link as `[the spec](https://…)` and a heading the model wrote as
 * `### Rủi ro`. It was Markdown source, shown to somebody who never asked to see any.
 *
 * The parse is [`toDoc`], the converter the note editor already round-trips through, so there is one
 * implementation of "what does this file say" with one set of tests behind it. This module is only
 * the drawing: no editing, no ProseMirror, and none of the 600 kB of editor on a screen being read.
 *
 * ## Checkboxes
 *
 * A task list is drawn with real checkboxes and they work, because a checkbox that does nothing is a
 * worse lie than the plain text this replaced. A line the vault gave an id — `<!-- id:01T1 -->`, the
 * same id the task board moves — is tickable, and ticking it writes to the same file the board
 * writes to. A line with no id cannot be addressed, so its box is disabled and says why.
 */

interface Props {
  /** The section body as it is on disk, comments and all. See [`clean`]. */
  markdown: string;
  /** Turns a vault link such as `attachments/x.png` into something a browser can fetch. */
  resolveImage?: (link: string) => string;
  /** Called when a checkbox with an id is ticked. Without it every box is read-only. */
  onToggleTask?: (id: string, done: boolean) => void;
  /**
   * Ticks in flight: the id, and the state the user asked for.
   *
   * Drawn in preference to what is on disk. A checkbox is a controlled input whose truth is the
   * file, and the file does not change until the daemon has written it and this screen has read it
   * back — around a second. Without this the box springs back under the pointer and the click
   * reads as ignored, which is the one thing a checkbox must never do.
   */
  pending?: ReadonlyMap<string, boolean>;
  className?: string;
}

/** The machine-readable state the vault carries in comments, removed before a person sees it. */
function clean(text: string): string {
  return text.replace(/<!--[\s\S]*?-->/g, "").replace(/[ \t]+$/g, "");
}

/** The task id on a line, when the vault gave it one. */
function idOf(text: string): string | null {
  return /<!--[^>]*\bid:(\S+?)[\s-]/.exec(text)?.[1] ?? null;
}

/** Everything a node and its children say, before anything is stripped. */
function plain(node: JSONContent): string {
  if (typeof node.text === "string") return node.text;
  return (node.content ?? []).map(plain).join("");
}

interface Shared {
  resolveImage?: (link: string) => string;
  onToggleTask?: (id: string, done: boolean) => void;
  pending?: ReadonlyMap<string, boolean>;
}

export function Markdown({ markdown, className, ...shared }: Props) {
  const doc = toDoc(markdown);
  return (
    <div className={cn("space-y-3 leading-relaxed", className)}>
      {(doc.content ?? []).map((node, at) => (
        <Block key={at} node={node} {...shared} />
      ))}
    </div>
  );
}

function Block({ node, ...shared }: Shared & { node: JSONContent }): ReactNode {
  const children = (node.content ?? []).map((child, at) => (
    <Block key={at} node={child} {...shared} />
  ));

  switch (node.type) {
    case "heading":
      // Always an `<h4>`, whatever the file wrote: these sections are already under the card's own
      // heading, and an `<h1>` inside one is a document outline that lies to a screen reader.
      return (
        <h4
          className={cn(
            "mt-4 mb-1 font-semibold first:mt-0",
            Number(node.attrs?.level ?? 1) > 2 ? "text-sm" : "text-base",
          )}
        >
          <Inline nodes={node.content} {...shared} />
        </h4>
      );
    case "paragraph":
      return (
        <p>
          <Inline nodes={node.content} {...shared} />
        </p>
      );
    case "bulletList":
      return <ul className="ms-5 list-disc space-y-1">{children}</ul>;
    case "orderedList":
      return (
        <ol className="ms-5 list-decimal space-y-1" start={Number(node.attrs?.start ?? 1)}>
          {children}
        </ol>
      );
    case "listItem":
      return <Item node={node} {...shared} />;
    case "taskList":
      return <ul className="space-y-1.5">{children}</ul>;
    case "taskItem":
      return <TaskItem node={node} {...shared} />;
    case "codeBlock":
      return (
        <pre className="border-line bg-bg-soft text-meta overflow-x-auto rounded-lg border p-3">
          <code>{(node.content ?? []).map((child) => child.text ?? "").join("")}</code>
        </pre>
      );
    case "blockquote":
      return (
        <blockquote className="border-line text-fg-dim space-y-2 border-s-2 ps-3">
          {children}
        </blockquote>
      );
    case "horizontalRule":
      return <hr className="border-line" />;
    case "table":
      return (
        <div className="overflow-x-auto">
          <table className="border-line w-full border-collapse text-sm">
            <tbody>{children}</tbody>
          </table>
        </div>
      );
    case "tableRow":
      return <tr>{children}</tr>;
    case "tableHeader":
    case "tableCell": {
      const Cell = node.type === "tableHeader" ? "th" : "td";
      const align: unknown = node.attrs?.align;
      return (
        <Cell
          className={cn(
            "border-line border px-2.5 py-1.5 align-top",
            node.type === "tableHeader" && "bg-bg-soft font-medium",
            align === "center" && "text-center",
            align === "right" && "text-right",
            align === "left" && "text-left",
          )}
        >
          {/* A cell's paragraphs, without the paragraphs: a one-line cell should be one line. */}
          {(node.content ?? []).map((child, at) => (
            <Inline key={at} nodes={child.content} {...shared} />
          ))}
        </Cell>
      );
    }
    default:
      return (
        <p>
          <Inline nodes={node.content} {...shared} />
        </p>
      );
  }
}

/**
 * One bullet, with its first line beside the marker.
 *
 * A list item holds blocks, and rendering the first as a `<p>` puts the text on the line under the
 * bullet. Anything after the first — a nested list, a second paragraph — does belong under it.
 */
function Item({ node, ...shared }: Shared & { node: JSONContent }) {
  const [first, ...rest] = node.content ?? [];
  return (
    <li>
      <Inline nodes={first?.content} {...shared} />
      {rest.length > 0 && (
        <div className="mt-1 space-y-1">
          {rest.map((child, at) => (
            <Block key={at} node={child} {...shared} />
          ))}
        </div>
      )}
    </li>
  );
}

function TaskItem({ node, ...shared }: Shared & { node: JSONContent }) {
  const t = useT();
  const { onToggleTask, pending } = shared;
  const [first, ...rest] = node.content ?? [];
  const id = idOf(plain(node));
  const asked = id === null ? undefined : pending?.get(id);
  const checked = asked ?? Boolean(node.attrs?.checked);
  const busy = asked !== undefined;
  const live = id !== null && onToggleTask !== undefined;

  return (
    <li>
      {/* A label rather than a checkbox with an `aria-label` copy of the text beside it: the text
          is the label, and this way the whole line is a hit target. */}
      <label className={cn("flex items-start gap-2", live && !busy && "cursor-pointer")}>
        <input
          type="checkbox"
          checked={checked}
          disabled={!live || busy}
          // A box nobody can tick explains itself once, rather than looking broken.
          title={live ? undefined : t("meeting.task_not_tickable")}
          onChange={() => id && onToggleTask?.(id, !checked)}
          className="accent-accent mt-1 size-3.5 shrink-0 disabled:opacity-40"
        />
        <span className={cn("min-w-0 flex-1", checked && "text-fg-faint line-through")}>
          <Inline nodes={first?.content} {...shared} />
        </span>
      </label>
      {rest.length > 0 && (
        <div className="mt-1 ms-5.5 space-y-1">
          {rest.map((child, at) => (
            <Block key={at} node={child} {...shared} />
          ))}
        </div>
      )}
    </li>
  );
}

function Inline({ nodes, resolveImage }: Shared & { nodes?: JSONContent[] }) {
  return (
    <>
      {(nodes ?? []).map((node, at) => {
        if (node.type === "hardBreak") return <br key={at} />;
        if (node.type === "image") {
          const src = String(node.attrs?.src ?? "");
          return (
            <img
              key={at}
              src={resolveImage ? resolveImage(src) : src}
              alt={String(node.attrs?.alt ?? "")}
              className="my-2 max-w-full rounded-lg"
            />
          );
        }
        const text = clean(node.text ?? "");
        if (text === "") return null;
        return (
          <Marked key={at} marks={node.marks}>
            {text}
          </Marked>
        );
      })}
    </>
  );
}

function Marked({
  marks,
  children,
}: {
  marks?: { type: string; attrs?: Record<string, unknown> }[];
  children: ReactNode;
}) {
  let out = children;
  for (const mark of marks ?? []) {
    if (mark.type === "bold") out = <strong className="font-semibold">{out}</strong>;
    else if (mark.type === "italic") out = <em>{out}</em>;
    else if (mark.type === "strike") out = <s className="text-fg-faint">{out}</s>;
    else if (mark.type === "code")
      out = <code className="bg-bg-soft rounded px-1 py-0.5 text-[0.9em]">{out}</code>;
    else if (mark.type === "link") {
      // Only a string is a link. Anything else in that attribute came from a document this
      // converter did not write, and `[object Object]` is not an address.
      const href = typeof mark.attrs?.href === "string" ? mark.attrs.href : "";
      // `noopener noreferrer`: a link out of a local app into whatever a meeting mentioned, and
      // neither the opener nor the referrer is that site's business.
      out = (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="text-accent hover:underline"
        >
          {out}
        </a>
      );
    }
  }
  return <>{out}</>;
}
