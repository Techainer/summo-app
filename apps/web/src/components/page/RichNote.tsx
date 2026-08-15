import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import {
  CheckSquare,
  Code,
  Heading1,
  Heading2,
  Heading3,
  List,
  ListOrdered,
  Minus,
  Quote,
  Type,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "../../lib/cn";
import { useT } from "../../i18n/context";
import { faithful, same, toDoc, toMarkdown } from "../../lib/markdown";

/**
 * A note you can format, over a file that is still Markdown.
 *
 * The vault is a folder a person opens in Obsidian, greps and backs up, and the objection to a rich
 * editor has always been that one stores its own shape and quietly breaks that the first time it
 * round-trips something it does not understand. So this does not store a shape: the document is
 * converted to Markdown on every change and that is what is saved.
 *
 * ## The guard
 *
 * Two checks, and the second is the one that matters.
 *
 * [`faithful`] is a cheap pre-check on the text before an editor exists. Then, once the document
 * has been through ProseMirror — which coerces whatever it is given to its own schema, and is
 * therefore the part that can lose something the converter got right — the result is serialized and
 * compared to the text that was loaded. A mismatch means this note is not one this editor can hold
 * without changing it, and the caller is told so and shows the plain textarea instead.
 *
 * Falling back is the guarantee, not a failure to be minimised. A note with a table opens as text
 * and keeps its table.
 *
 * ## Why the checkboxes matter
 *
 * A to-do here is `- [ ] @ngoc …` in the file, which is the line `summo_vault::tasks` already
 * parses. Ticking a box in a note ticks it on the task board, in the report and in the agent's
 * view, because there is one representation and it is the text.
 */
export function RichNote({
  markdown,
  onChange,
  onUnsupported,
  className,
}: {
  /** The note as it is on disk, below its title. */
  markdown: string;
  onChange: (markdown: string) => void;
  /** Called once if this note cannot be held without changing it. */
  onUnsupported: () => void;
  className?: string;
}) {
  const t = useT();
  const [slash, setSlash] = useState(false);
  // The last text handed out, so a change event that produces what we were given does not report
  // an edit — which would mark a note unsaved the moment it was opened.
  const written = useRef(markdown);

  const editor = useEditor({
    extensions: useMemo(
      () => [
        StarterKit.configure({
          // Three levels. A note is not a book, and the converter writes `#`, `##`, `###` only —
          // offering a fourth would produce a line this editor could not read back.
          heading: { levels: [1, 2, 3] },
          link: false,
        }),
        TaskList,
        // Nested, because a checklist that cannot have sub-steps is one people abandon.
        TaskItem.configure({ nested: true }),
        Link.configure({ openOnClick: false, autolink: false }),
        Placeholder.configure({ placeholder: t("notes.slash_hint") }),
      ],
      [t],
    ),
    content: toDoc(markdown),
    editorProps: {
      attributes: {
        // `font-reading` and a measure: a note is prose, and prose at full window width is prose
        // nobody re-reads.
        class: "font-reading text-body leading-relaxed outline-none",
        "aria-label": t("notes.body"),
      },
    },
    onUpdate: ({ editor: live }) => {
      const next = toMarkdown(live.getJSON());
      if (same(next, written.current)) return;
      written.current = next;
      onChange(next.trim());
    },
  });

  // The check that decides whether this editor may be used at all.
  //
  // After the content has been through ProseMirror, not before: the schema coerces what it is
  // given, and that coercion is exactly the step a converter test cannot see. Reported through a
  // ref so a re-render cannot raise it twice and put the caller in a loop.
  const told = useRef(false);
  useEffect(() => {
    if (!editor || told.current) return;
    if (faithful(markdown) && same(toMarkdown(editor.getJSON()), markdown)) return;
    told.current = true;
    onUnsupported();
  }, [editor, markdown, onUnsupported]);

  // `/` on an empty line opens the block menu, the way it does in Notion. Read from the document
  // rather than from the keystroke, so deleting back to a bare `/` opens it too.
  useEffect(() => {
    if (!editor) return;
    const check = () => {
      const { $from, empty } = editor.state.selection;
      const line = $from.parent.textContent;
      setSlash(empty && line === "/" && $from.parent.type.name === "paragraph");
    };
    editor.on("selectionUpdate", check);
    editor.on("update", check);
    return () => {
      editor.off("selectionUpdate", check);
      editor.off("update", check);
    };
  }, [editor]);

  const run = useCallback(
    (apply: (chain: ReturnType<Editor["chain"]>) => void) => {
      if (!editor) return;
      // The `/` itself is not content; it is how the menu was asked for.
      const chain = editor.chain().focus().deleteRange({
        from: editor.state.selection.$from.start(),
        to: editor.state.selection.from,
      });
      apply(chain);
      chain.run();
      setSlash(false);
    },
    [editor],
  );

  const blocks = useMemo(
    () =>
      [
        { key: "text", icon: Type, apply: (c: ReturnType<Editor["chain"]>) => c.setParagraph() },
        {
          key: "h1",
          icon: Heading1,
          apply: (c: ReturnType<Editor["chain"]>) => c.setHeading({ level: 1 }),
        },
        {
          key: "h2",
          icon: Heading2,
          apply: (c: ReturnType<Editor["chain"]>) => c.setHeading({ level: 2 }),
        },
        {
          key: "h3",
          icon: Heading3,
          apply: (c: ReturnType<Editor["chain"]>) => c.setHeading({ level: 3 }),
        },
        {
          key: "todo",
          icon: CheckSquare,
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleTaskList(),
        },
        {
          key: "bullet",
          icon: List,
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleBulletList(),
        },
        {
          key: "ordered",
          icon: ListOrdered,
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleOrderedList(),
        },
        {
          key: "quote",
          icon: Quote,
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleBlockquote(),
        },
        { key: "code", icon: Code, apply: (c: ReturnType<Editor["chain"]>) => c.toggleCodeBlock() },
        {
          key: "divider",
          icon: Minus,
          apply: (c: ReturnType<Editor["chain"]>) => c.setHorizontalRule(),
        },
      ] as const,
    [],
  );

  return (
    <div className={cn("relative min-h-0 flex-1 overflow-y-auto px-6 py-5", className)}>
      <EditorContent editor={editor} />

      {/* In the flow under the cursor's block rather than positioned over it. A menu placed by
          coordinates has to be re-placed on every scroll, resize and reflow of the document
          underneath it, and gets it wrong at the bottom of the pane — where a person typing a long
          note spends all of their time. */}
      {slash && (
        <ul
          data-testid="block-menu"
          aria-label={t("notes.insert")}
          className="border-line bg-bg-raised mt-1 w-56 rounded-[var(--radius-card)] border py-1 shadow-[var(--shadow-pop)]"
        >
          {blocks.map((block) => (
            <li key={block.key}>
              <button
                type="button"
                onClick={() => run(block.apply)}
                className="text-fg-dim hover:bg-bg-soft hover:text-fg text-meta flex w-full items-center gap-2 px-3 py-1.5 text-start"
              >
                <block.icon aria-hidden="true" className="size-3.5 shrink-0" />
                {t(`notes.block_${block.key}`)}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
