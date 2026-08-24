import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { BubbleMenu } from "@tiptap/react/menus";
import { DragHandle } from "@tiptap/extension-drag-handle-react";
import { TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Highlight from "@tiptap/extension-highlight";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import {
  Bold,
  CheckSquare,
  Code,
  FileText,
  GripVertical,
  Heading1,
  Heading2,
  Heading3,
  Image as ImageIcon,
  Italic,
  Link2,
  List,
  ListOrdered,
  Minus,
  Quote,
  Strikethrough,
  Table as TableIcon,
  Trash2,
  Type,
  Highlighter,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "../../lib/cn";
import { useT } from "../../i18n/context";
import { faithful, same, toDoc, toMarkdown } from "../../lib/markdown";
import { fold } from "../../lib/palette";
import { PageLink, TABLE, VaultImage } from "./blocks";

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
 * Falling back is the guarantee, not a failure to be minimised. A note with a footnote opens as
 * text and keeps its footnote.
 *
 * ## Why the checkboxes matter
 *
 * A to-do here is `- [ ] @ngoc …` in the file, which is the line `summo_vault::tasks` already
 * parses. Ticking a box in a note ticks it on the task board, in the report and in the agent's
 * view, because there is one representation and it is the text.
 */
/**
 * The languages a code block can be tagged with.
 *
 * A list rather than a free-text field: the tag goes into the fence, every reader of the file has
 * its own set of names, and a typo means no highlighting anywhere with nothing on screen to say
 * why. These are the ones a meeting note actually carries.
 */
const CODE_LANGUAGES = [
  "bash",
  "json",
  "yaml",
  "sql",
  "python",
  "javascript",
  "typescript",
  "rust",
  "go",
  "java",
  "html",
  "css",
  "markdown",
] as const;

export function RichNote({
  markdown,
  onChange,
  onUnsupported,
  onUpload,
  resolveImage,
  onNewSubpage,
  onOpenPage,
  autofocus = false,
  className,
}: {
  /** The note as it is on disk, below its title. */
  markdown: string;
  onChange: (markdown: string) => void;
  /** Called once if this note cannot be held without changing it. */
  onUnsupported: () => void;
  /** Store a picture and answer with the vault link the note should carry. */
  onUpload?: (file: File) => Promise<string>;
  /** A vault link to something this browser can fetch. */
  resolveImage?: (link: string) => string;
  /**
   * Put the caret in the note as it opens.
   *
   * For a note that was made a moment ago and has nothing in it. Pressing Viết, watching a blank
   * page arrive and then having to click it before typing is a step nobody would design; every
   * other editor puts the cursor where you are about to type. Not for a note with words already in
   * it — stealing focus from somebody who came to read is worse than the click it saves.
   */
  autofocus?: boolean;
  /**
   * Make a page inside this one and answer with what to link to.
   *
   * Optional, because the editor is also mounted where there is nothing to be inside of — the
   * absence removes the row from the menu rather than leaving one that does nothing.
   */
  onNewSubpage?: () => Promise<{ id: string; title: string } | null>;
  /** Follow a link to another page in this vault. */
  onOpenPage?: (id: string) => void;
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
  /** Where the cursor is, for the parts of the toolbar that only apply inside a table. */
  const [inTable, setInTable] = useState(false);
  const [busy, setBusy] = useState(false);
  // The last text handed out, so a change event that produces what we were given does not report
  // an edit — which would mark a note unsaved the moment it was opened.
  const written = useRef(markdown);
  const editorRef = useRef<Editor | null>(null);

  /**
   * Put a picture in the note.
   *
   * One path for the file input, for a paste and for a drop, because three ways of adding a picture
   * that each upload it slightly differently is three sets of the same bug.
   */
  const insert = useCallback(
    async (live: Editor, file: File) => {
      if (!onUpload) return;
      setBusy(true);
      try {
        const link = await onUpload(file);
        live.chain().focus().setImage({ src: link, alt: file.name }).run();
      } catch {
        // Deliberately silent here and reported by the caller's own error bar: this component has
        // no place to put a message, and a picture that failed to upload is a picture that is not
        // in the note — which the user can see.
      } finally {
        setBusy(false);
      }
    },
    [onUpload],
  );

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
        PageLink.configure({ openOnClick: false, autolink: false }),
        ...TABLE,
        VaultImage.configure({
          // Inline, because Markdown puts a picture inside a paragraph and pretending otherwise
          // would mean writing a blank line into the file that nobody typed.
          inline: true,
          allowBase64: false,
          resolve: resolveImage ?? ((link: string) => link),
        }),
        // `==text==`, which Obsidian, Bear and Typora all read. Multicolour is deliberately off:
        // a colour is a fact this file cannot carry, and a highlight that turns grey on the next
        // open is worse than one colour that always survives.
        Highlight.configure({ multicolor: false }),
        Placeholder.configure({ placeholder: t("notes.slash_hint") }),
      ],
      [resolveImage, t],
    ),
    content: toDoc(markdown),
    autofocus: autofocus ? "end" : false,
    editorProps: {
      attributes: {
        // `font-reading` and a measure: a note is prose, and prose at full window width is prose
        // nobody re-reads.
        class: "font-reading text-body leading-relaxed outline-none",
        "aria-label": t("notes.body"),
        // A `contenteditable` div is not a text field to anything reading the page: without these
        // two attributes assistive technology announces a named group and nothing typeable, and
        // `getByRole("textbox")` finds no editor.
        role: "textbox",
        "aria-multiline": "true",
      },
      // A screenshot on the clipboard is the way most pictures reach a note, and a picture that has
      // to be saved to disk first is one people do not bother with.
      handlePaste: (view, event) => {
        const file = [...(event.clipboardData?.files ?? [])].find((f) =>
          f.type.startsWith("image/"),
        );
        if (!file || !onUpload || !view.editable || !editorRef.current) return false;
        event.preventDefault();
        void insert(editorRef.current, file);
        return true;
      },
      handleDrop: (view, event) => {
        const file = [...(event.dataTransfer?.files ?? [])].find((f) =>
          f.type.startsWith("image/"),
        );
        if (!file || !onUpload || !editorRef.current) return false;
        event.preventDefault();
        // Where it was dropped, not where the cursor was. A picture that lands at the top of the
        // note because that is where the caret happened to be is a picture in the wrong place.
        const landed = view.posAtCoords({ left: event.clientX, top: event.clientY });
        if (landed) {
          view.dispatch(
            view.state.tr.setSelection(TextSelection.near(view.state.doc.resolve(landed.pos))),
          );
        }
        void insert(editorRef.current, file);
        return true;
      },
    },
    onUpdate: ({ editor: live }) => {
      const next = toMarkdown(live.getJSON());
      if (same(next, written.current)) return;
      written.current = next;
      onChange(next.trim());
    },
  });

  // The editor, reachable from the handlers above — which are built when the editor is created and
  // therefore cannot close over it. Assigned in an effect rather than during render: a paste or a
  // drop is a user gesture and cannot arrive before the first commit.
  useEffect(() => {
    editorRef.current = editor;
  }, [editor]);

  /**
   * Ask for a picture from the disk.
   *
   * An input made for the occasion rather than a hidden one kept in the tree. A file input that is
   * always there is a control in the document that nothing may show, whose only purpose is to be
   * clicked by something else — and keeping a ref to it made every list this component builds a
   * list something might read a ref out of.
   */
  const pick = useCallback(() => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/jpeg,image/gif,image/webp";
    input.addEventListener("change", () => {
      const file = input.files?.[0];
      if (file && editor) void insert(editor, file);
    });
    input.click();
  }, [editor, insert]);

  /**
   * A page inside this one, and a link to it where the cursor is.
   *
   * The page is made first and linked with the id it came back with, so a link in the file always
   * points at something that exists.
   */
  const subpage = useCallback(() => {
    void (async () => {
      const page = await onNewSubpage?.();
      if (!page || !editor) return;
      // A link, because that is what a sub-page *is* in the file: `[Tên](/pages/id)` reads
      // correctly in Obsidian and needs nothing from this app to mean something.
      editor
        .chain()
        .focus()
        .insertContent([
          {
            type: "text",
            text: page.title,
            marks: [{ type: "link", attrs: { href: `/pages/${page.id}` } }],
          },
        ])
        // Off the end of the link, so the next thing typed is not part of it.
        .unsetMark("link")
        .run();
    })();
  }, [editor, onNewSubpage]);

  // The check that decides whether this editor may be used at all.
  //
  // After the content has been through ProseMirror, not before: the schema coerces what it is
  // given, and that coercion is exactly the step a converter test cannot see. A ragged table is the
  // clearest case — the converter reproduces one exactly, and ProseMirror squares it up. Reported
  // through a ref so a re-render cannot raise it twice and put the caller in a loop.
  // The check that decides whether this editor may be used at all — and *when* it may be asked.
  //
  // After the content has been through ProseMirror, not before: the schema coerces what it is
  // given, and that coercion is exactly the step a converter test cannot see. A ragged table is the
  // clearest case — the converter reproduces one exactly, and ProseMirror squares it up.
  //
  // Asked only when `markdown` changes, which is the only moment the two are meant to agree. It
  // used to depend on `onUnsupported` as well, and both callers pass a fresh arrow on every render
  // — so this ran on every render of a page that rerenders on every transcript line. A keystroke
  // mutates the editor first and reaches this prop one render later, so any transcript line landing
  // in that window compared the new document against the old text, found them different, and
  // handed the note to the plain-text fallback for good. Measured: one line of ordinary Vietnamese
  // typed into an empty meeting note, about one time in five.
  //
  // The callback goes through a ref rather than into the dependency array. Memoising it at the call
  // sites would fix today's two callers and quietly wait for the third.
  const told = useRef(false);
  const unsupported = useRef(onUnsupported);
  useEffect(() => {
    unsupported.current = onUnsupported;
  });
  useEffect(() => {
    if (!editor || told.current) return;
    if (faithful(markdown) && same(toMarkdown(editor.getJSON()), markdown)) return;
    told.current = true;
    // Through the ref, so `onUnsupported` is not a dependency and a caller passing a fresh arrow
    // cannot turn this into a per-render check again.
    unsupported.current();
  }, [editor, markdown]);

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
      setInTable(editor.isActive("table"));
    };
    editor.on("selectionUpdate", check);
    editor.on("update", check);
    return () => {
      editor.off("selectionUpdate", check);
      editor.off("update", check);
    };
  }, [editor]);

  /**
   * Apply a block, having first removed the `/` that asked for it.
   *
   * `act` rather than a chain for the two blocks that have to go and fetch something — a picture
   * from the disk, a page from the daemon. Both still delete the query first, so a slash command
   * that opens a file dialog does not leave `/anh` in the paragraph while the dialog is up.
   */
  const run = useCallback(
    (block: BlockAction) => {
      if (!editor) return;
      const chain = editor.chain().focus().deleteRange({
        from: editor.state.selection.$from.start(),
        to: editor.state.selection.from,
      });
      if (block.apply) block.apply(chain);
      chain.run();
      setQuery(null);
      block.act?.();
    },
    [editor],
  );

  const blocks: BlockAction[] = useMemo(
    () =>
      [
        { key: "text", icon: Type, apply: (c: Chain) => c.setParagraph() },
        { key: "h1", icon: Heading1, apply: (c: Chain) => c.setHeading({ level: 1 }) },
        { key: "h2", icon: Heading2, apply: (c: Chain) => c.setHeading({ level: 2 }) },
        { key: "h3", icon: Heading3, apply: (c: Chain) => c.setHeading({ level: 3 }) },
        { key: "todo", icon: CheckSquare, apply: (c: Chain) => c.toggleTaskList() },
        { key: "bullet", icon: List, apply: (c: Chain) => c.toggleBulletList() },
        { key: "ordered", icon: ListOrdered, apply: (c: Chain) => c.toggleOrderedList() },
        { key: "quote", icon: Quote, apply: (c: Chain) => c.toggleBlockquote() },
        { key: "code", icon: Code, apply: (c: Chain) => c.toggleCodeBlock() },
        {
          key: "table",
          icon: TableIcon,
          // Three columns and a header row: the shape of nearly every table anybody starts, and one
          // fewer decision before there is something on the screen to edit.
          apply: (c: Chain) => c.insertTable({ rows: 3, cols: 3, withHeaderRow: true }),
        },
        ...(onUpload ? [{ key: "image", icon: ImageIcon, act: pick }] : []),
        ...(onNewSubpage ? [{ key: "subpage", icon: FileText, act: subpage }] : []),
        { key: "divider", icon: Minus, apply: (c: Chain) => c.setHorizontalRule() },
      ] as BlockAction[],
    [onNewSubpage, onUpload, pick, subpage],
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
        if (block) run(block);
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
        { key: "bold", icon: Bold, is: "bold", apply: (c: Chain) => c.toggleBold() },
        { key: "italic", icon: Italic, is: "italic", apply: (c: Chain) => c.toggleItalic() },
        { key: "strike", icon: Strikethrough, is: "strike", apply: (c: Chain) => c.toggleStrike() },
        { key: "code", icon: Code, is: "code", apply: (c: Chain) => c.toggleCode() },
        {
          key: "highlight",
          icon: Highlighter,
          is: "highlight",
          apply: (c: Chain) => c.toggleHighlight(),
        },
      ] as const,
    [],
  );

  /** What can be done to the table the cursor is in. */
  const tableActions = useMemo(
    () =>
      [
        { key: "row_after", apply: (c: Chain) => c.addRowAfter() },
        { key: "row_delete", apply: (c: Chain) => c.deleteRow() },
        { key: "column_after", apply: (c: Chain) => c.addColumnAfter() },
        { key: "column_delete", apply: (c: Chain) => c.deleteColumn() },
        { key: "header_row", apply: (c: Chain) => c.toggleHeaderRow() },
        { key: "delete", icon: Trash2, apply: (c: Chain) => c.deleteTable() },
      ] as const,
    [],
  );

  return (
    // The two rules disabled here are about a *control* made out of a `div`, and this is not one:
    // it is the editor's scroll container, and the handler is delegation for links ProseMirror
    // draws inside a `contenteditable`. There is no keyboard equivalent to add — a link in an
    // editor is not in the tab order, because Tab there inserts a tab. The keyboard route to a
    // sub-page is the slash menu and the sidebar, both of which are real controls.
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div
      className={cn("relative min-h-0 flex-1 overflow-y-auto px-6 py-5", className)}
      // A sub-page is a link, and a link inside an editor does nothing when you click it — the
      // click sets the caret, which is right for a URL and wrong for a page in this vault. Caught
      // here rather than by turning `openOnClick` back on, because that would also follow every
      // `https://` in the note into whatever the shell decides to do with it.
      onClick={(event) => {
        // Clicking the empty space under the last line puts the caret at the end, the way every
        // page-shaped editor does. Without it a note with one line in it is a page where four
        // hundred pixels of blank paper do nothing when you click them — which reads, correctly,
        // as an editor that is not accepting typing.
        const target = event.target as HTMLElement;
        if (editor && (target === event.currentTarget || target.dataset.blank === "yes")) {
          editor.chain().focus("end").run();
          return;
        }
        if (!onOpenPage) return;
        const link = target.closest?.("a[data-page]");
        const href = link?.getAttribute("href") ?? "";
        const page = /^\/pages\/([^/?#]+)/.exec(href)?.[1];
        if (!page) return;
        event.preventDefault();
        onOpenPage(decodeURIComponent(page));
      }}
    >
      {/* Sticky in the flow rather than floating beside the cursor. A menu placed by coordinates
          has to be re-placed on every scroll and reflow of the document underneath it, and a table
          is exactly the block a person scrolls while editing. */}
      {inTable && (
        <div
          data-testid="table-tools"
          role="toolbar"
          aria-label={t("notes.table_tools")}
          className="border-line bg-bg-raised sticky top-0 z-10 mb-2 flex flex-wrap items-center gap-1 rounded-[var(--radius-card)] border p-1 shadow-[var(--shadow-sm)]"
        >
          {tableActions.map((action) => (
            <button
              key={action.key}
              type="button"
              onClick={() => editor && action.apply(editor.chain().focus()).run()}
              className={cn(
                "text-micro rounded-[var(--radius-pill)] px-2 py-1 transition-colors",
                action.key === "delete"
                  ? "text-danger hover:bg-danger-soft"
                  : "text-fg-dim hover:bg-bg-soft hover:text-fg",
              )}
            >
              {t(`notes.table_${action.key}`)}
            </button>
          ))}
        </div>
      )}

      {/* The gutter handle. A block you cannot pick up is a block you rewrite to move, and rewriting
          a paragraph to move it is what makes people paste notes into another app. */}
      {editor && (
        <DragHandle editor={editor} nested>
          <div
            aria-hidden="true"
            className="text-fg-faint hover:bg-bg-soft hover:text-fg-dim mr-1 grid size-6 cursor-grab place-items-center rounded active:cursor-grabbing"
          >
            <GripVertical className="size-4" />
          </div>
        </DragHandle>
      )}

      <EditorContent editor={editor} />

      {/* The rest of the page. A click here lands on the wrapper above and puts the caret at the
          end; without something to catch it, the blank half of a short note is dead space. */}
      <div data-blank="yes" aria-hidden="true" className="min-h-24 flex-1 cursor-text" />

      {busy && (
        <p role="status" className="text-fg-faint text-micro mt-2">
          {t("notes.uploading")}
        </p>
      )}

      {/* Which language a code block is in.
          The converter has always written ```` ```rust ```` and read it back, and nothing in the
          interface could set it — so every block somebody typed came back as plain text with no
          highlighting and no way to ask for any. A select, in the flow, only while the caret is in
          a code block. */}
      {editor?.isActive("codeBlock") && (
        <label className="border-line bg-bg-raised sticky bottom-2 z-10 mt-2 flex w-fit items-center gap-2 rounded-[var(--radius-pill)] border px-3 py-1.5 shadow-[var(--shadow-sm)]">
          <span className="text-fg-faint text-micro">{t("notes.code_language")}</span>
          <select
            aria-label={t("notes.code_language")}
            value={(editor.getAttributes("codeBlock").language as string | null) ?? ""}
            onChange={(event) =>
              editor
                .chain()
                .focus()
                .updateAttributes("codeBlock", { language: event.target.value || null })
                .run()
            }
            className="text-micro bg-transparent outline-none"
          >
            <option value="">{t("notes.code_plain")}</option>
            {CODE_LANGUAGES.map((language) => (
              <option key={language} value={language}>
                {language}
              </option>
            ))}
          </select>
        </label>
      )}

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
                onClick={() => run(block)}
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

type Chain = ReturnType<Editor["chain"]>;

/** One row of the block menu: a formatting change, or something that has to go and fetch first. */
interface BlockAction {
  key: string;
  icon: typeof Type;
  apply?: (chain: Chain) => void;
  act?: () => void;
}
