import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { BubbleMenu } from "@tiptap/react/menus";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import {
  Bold,
  CheckSquare,
  Code,
  Heading1,
  Heading2,
  Heading3,
  Italic,
  Link2,
  List,
  ListOrdered,
  Minus,
  Quote,
  Strikethrough,
  Type,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "../../lib/cn";
import { useT } from "../../i18n/context";
import { faithful, same, toDoc, toMarkdown } from "../../lib/markdown";
import { fold } from "../../lib/palette";

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
  /**
   * What has been typed after `/`, or `null` when the menu is closed.
   *
   * A string rather than a flag, because a menu of ten blocks that cannot be narrowed is a menu you
   * read every time. `/` then `vi` should be one keystroke away from a to-do, the way it is in
   * Notion — and the same string is what closes the menu again when it matches nothing, so `/etc`
   * in a note is text and not a stuck popup.
   */
  const [query, setQuery] = useState<string | null>(null);
  /** Which row the keyboard is on. Reset whenever the query changes the list under it. */
  const [at, setAt] = useState(0);
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

  // `/` at the start of an empty paragraph opens the block menu, the way it does in Notion. Read
  // from the *document* rather than from the keystroke, so deleting back to a bare `/` opens it
  // again and typing past a match closes it.
  useEffect(() => {
    if (!editor) return;
    const check = () => {
      const { $from, empty } = editor.state.selection;
      const line = $from.parent.textContent;
      const asking =
        empty && $from.parent.type.name === "paragraph" && $from.parentOffset === line.length
          ? /^\/(\S*)$/.exec(line)?.[1]
          : undefined;
      setQuery(asking ?? null);
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
      // The `/` and whatever was typed after it are not content; they are how the menu was asked
      // for and narrowed.
      const chain = editor.chain().focus().deleteRange({
        from: editor.state.selection.$from.start(),
        to: editor.state.selection.from,
      });
      apply(chain);
      chain.run();
      setQuery(null);
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

  /**
   * The blocks this query names, in menu order.
   *
   * Folded, so `/vi` finds `Việc cần làm` from a keyboard with no Vietnamese layout — the same
   * reason the command palette folds. An empty result closes the menu rather than showing an empty
   * box: `/etc/passwd` typed into a note is text, and a popup that will not go away is worse than
   * one that never opened.
   */
  const shown = useMemo(() => {
    if (query === null) return [];
    const wanted = fold(query);
    return blocks.filter((block) => fold(t(`notes.block_${block.key}`)).includes(wanted));
  }, [blocks, query, t]);

  const open = shown.length > 0;
  // The list under the cursor changed, so the row the keyboard is on is about to mean something
  // else. Clamped during render rather than in an effect: an effect would paint one frame with the
  // highlight on a row that is no longer there.
  const chosen = Math.min(at, Math.max(shown.length - 1, 0));

  // Arrows, Enter and Escape belong to the menu while it is open.
  //
  // On `document`, in the capture phase, and that is not a stylistic choice. ProseMirror binds its
  // own `keydown` to the editor node at creation; a second listener on that same node runs *after*
  // it, whatever the capture flag says, because at the target both phases fire in registration
  // order. So Enter split the paragraph and the menu's `preventDefault` arrived too late — the
  // block was never applied and `/vi` stayed in the document as text.
  //
  // Not `editorProps.handleKeyDown` either: that is fixed when the editor is created and would
  // close over the first render's empty list for ever. Re-bound whenever the list or the highlight
  // changes, which costs one `addEventListener` per keystroke and removes every question about
  // stale state.
  useEffect(() => {
    if (!editor || !open) return;
    const dom = editor.view.dom;
    const onKey = (event: KeyboardEvent) => {
      if (!dom.contains(event.target as Node)) return;
      const move = { ArrowDown: 1, ArrowUp: -1 }[event.key];
      if (move !== undefined) {
        event.preventDefault();
        setAt((was) => (Math.min(was, shown.length - 1) + move + shown.length) % shown.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const block = shown[chosen];
        if (block) run(block.apply);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setQuery(null);
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [editor, open, shown, chosen, run]);

  const marks = useMemo(
    () =>
      [
        {
          key: "bold",
          icon: Bold,
          is: "bold",
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleBold(),
        },
        {
          key: "italic",
          icon: Italic,
          is: "italic",
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleItalic(),
        },
        {
          key: "strike",
          icon: Strikethrough,
          is: "strike",
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleStrike(),
        },
        {
          key: "code",
          icon: Code,
          is: "code",
          apply: (c: ReturnType<Editor["chain"]>) => c.toggleCode(),
        },
      ] as const,
    [],
  );

  return (
    <div className={cn("relative min-h-0 flex-1 overflow-y-auto px-6 py-5", className)}>
      <EditorContent editor={editor} />

      {/* Formatting where the text is, rather than in a toolbar at the top of a pane the user is
          not looking at. It appears only over a selection, which is the only moment any of it is
          applicable. */}
      {editor && (
        <BubbleMenu
          editor={editor}
          options={{ placement: "top" }}
          className="border-line bg-bg-raised flex items-center gap-0.5 rounded-[var(--radius-pill)] border p-1 shadow-[var(--shadow-pop)]"
        >
          {marks.map((mark) => (
            <button
              key={mark.key}
              type="button"
              aria-label={t(`notes.mark_${mark.key}`)}
              aria-pressed={editor.isActive(mark.is)}
              title={t(`notes.mark_${mark.key}`)}
              onClick={() => mark.apply(editor.chain().focus()).run()}
              className={cn(
                "grid size-7 place-items-center rounded-[var(--radius-pill)] transition-colors",
                editor.isActive(mark.is)
                  ? "bg-accent-soft text-accent"
                  : "text-fg-dim hover:bg-bg-soft hover:text-fg",
              )}
            >
              <mark.icon aria-hidden="true" className="size-3.5" />
            </button>
          ))}
          <span className="bg-line mx-0.5 h-4 w-px" aria-hidden="true" />
          <button
            type="button"
            aria-label={t("notes.mark_link")}
            aria-pressed={editor.isActive("link")}
            title={t("notes.mark_link")}
            onClick={() => {
              // The browser's own prompt. A link dialog is a form, a focus trap and a second place
              // to press Escape, for one field — and the one field is the whole feature.
              const now = (editor.getAttributes("link").href as string | undefined) ?? "";
              const href = window.prompt(t("notes.mark_link"), now);
              if (href === null) return;
              const chain = editor.chain().focus().extendMarkRange("link");
              if (href.trim() === "") chain.unsetLink().run();
              else chain.setLink({ href: href.trim() }).run();
            }}
            className={cn(
              "grid size-7 place-items-center rounded-[var(--radius-pill)] transition-colors",
              editor.isActive("link")
                ? "bg-accent-soft text-accent"
                : "text-fg-dim hover:bg-bg-soft hover:text-fg",
            )}
          >
            <Link2 aria-hidden="true" className="size-3.5" />
          </button>
        </BubbleMenu>
      )}

      {/* In the flow under the cursor's block rather than positioned over it. A menu placed by
          coordinates has to be re-placed on every scroll, resize and reflow of the document
          underneath it, and gets it wrong at the bottom of the pane — where a person typing a long
          note spends all of their time. */}
      {open && (
        <ul
          data-testid="block-menu"
          role="listbox"
          aria-label={t("notes.insert")}
          className="border-line bg-bg-raised mt-1 w-56 rounded-[var(--radius-card)] border py-1 shadow-[var(--shadow-pop)]"
        >
          {shown.map((block, index) => (
            <li key={block.key} role="option" aria-selected={index === chosen}>
              <button
                type="button"
                // The pointer moves the keyboard's highlight rather than fighting it: two rows
                // marked at once is two answers to "what does Enter do".
                onMouseEnter={() => setAt(index)}
                onClick={() => run(block.apply)}
                className={cn(
                  "text-meta flex w-full items-center gap-2 px-3 py-1.5 text-start transition-colors",
                  index === chosen ? "bg-accent-soft text-accent" : "text-fg-dim",
                )}
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
