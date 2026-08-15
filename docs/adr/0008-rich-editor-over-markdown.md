# 0008 — A rich editor, over a file that is still Markdown

**Status:** accepted
**Date:** 2026-08-15
**Supersedes:** [0005 — A textarea, not a rich editor](0005-plain-markdown-editor.md)

## The decision

Notes are edited in Tiptap. The document model is converted to Markdown on every change and that is
what is written to the vault — and a note the converter cannot reproduce **exactly** opens in the
plain textarea instead.

ADR 0005 refused a rich editor, and it named its own escape hatch:

> A rich editor whose document model _is_ the Markdown — one that treats unknown inline syntax as
> opaque text it must preserve byte-for-byte, and can prove it with round-trip tests over the
> constructs Summo actually writes. That is a different kind of editor from the ones available
> today, and if one appears the trade flips.

Nothing appeared. It was built, because the part that had to be written was never the editor.

## What makes it safe

The danger 0005 identified is exact and unchanged: the vault carries syntax no editor has heard of —
`- [ ] @ngoc … <!-- id:01J4 -->`, `<!-- summo:draft -->`, `<!-- seq:7 end:735.20 -->` — sitting
inline in content the editor does own. A lossy round trip does not crash. It saves.

Three things answer it, and only the third is unusual.

1. **`apps/web/src/lib/markdown.ts` is a converter for the shapes this vault uses**, not a general
   Markdown implementation. Roughly 500 readable lines, both directions, one file.
2. **`faithful()` compares writing back what was read against the file**, before an editor exists.
3. **The same comparison runs again after ProseMirror has seen the document**, which is the check
   that matters. The schema coerces whatever it is given, and that coercion is the step no converter
   test can observe. A table whose body row has fewer cells than its header is the clearest case:
   the converter reproduces it exactly, ProseMirror squares it up, and the note therefore opens as
   text with its ragged table intact.

Falling back is the guarantee, not a failure to be minimised away. A note with a footnote opens as a
textarea and keeps its footnote.

## What it can hold

Headings 1–3, paragraphs, bullet, ordered and task lists with nesting, block quotes, fenced code
with a language, thematic breaks, GFM tables with column alignment, and images. Inline: bold,
italic, code, strike, links.

Two of those needed a decision each.

**A cell holds one line in the file and more than one on screen.** The difference is written as
`<br>`, which every Markdown renderer already shows as a break inside a cell. A person who pressed
Enter in a cell keeps what they typed, and the file still reads correctly in Obsidian.

**Tables are not resizable.** A column width lives in the ProseMirror document and has nowhere to go
in Markdown, so a width somebody dragged would be written to a file that cannot hold it and be gone
on the next open — a control that appears to work and does not.

There is one cost worth stating plainly: a table whose pipes were lined up into a grid is written
back without the padding, once somebody edits the note. Treating the grid as content would send
every such note to the textarea instead, which is worse; opening one and leaving it alone still
writes nothing.

## Pictures

`vault/attachments/<hash>.<ext>`, linked as `attachments/<name>` relative to the vault root, served
by the daemon. Not a data URI: that turns a 400 kB screenshot into half a megabyte of base64 on one
unwrappable line, in a file the user greps.

The name is a hash of the bytes, so pasting one screenshot into a week of notes writes one file and
no upload can overwrite another. The format is read from the file's own magic bytes and never from
what the client said — the interface is served from the daemon's origin, so an SVG accepted here
would be script running in the app. Pictures nothing links to are swept by `summo serve`'s existing
prune, because the question "is this still used" is a whole-vault one.

## Pages inside pages

A sub-page is two facts stored in two places, on purpose:

- **The link** is ordinary Markdown in the parent — `[Chi tiết](/pages/<id>)` — so the note means
  something in any editor.
- **The parent** is `parent: <id>` in the child's frontmatter, so the tree survives the file being
  renamed, refiled or moved. A list on the parent would have to be rewritten on every reparenting,
  and two writes that can disagree is a tree that eventually does.

Nesting a page does not move its file. A folder is where a document _is_; a parent is what it is
_part of_. `summo_vault::library::set_parent` refuses a page nested under one of its own
descendants, because a loop in this tree is not a wrong drawing but an infinite one.

## What this costs

The editor's chunk is 201 kB gzipped and is fetched when a note is opened, never on the screen the
app starts on. 28 kB of that is a CRDT: `@tiptap/extension-drag-handle` depends on
`@tiptap/extension-collaboration`, which depends on Yjs. Nothing in Summo is collaborative. It rides
in the lazy chunk rather than being fought, and `apps/web/scripts/budget.mjs` is what noticed —
`manualChunks` had silently named it into the chunk every screen loads.

## Consequences

- `apps/web/src/components/page/NoteEditor.tsx` still owns the autosave and still shows a textarea;
  which one you get is decided by the document, once, when it is read.
- `apps/web/src/lib/markdown.test.ts` is the load-bearing test file for this decision. Anything the
  editor learns to hold has to arrive with a round trip that reproduces the file.
- 0005's closing line still holds: anyone wanting a different editor already has one, on the same
  files, at the same time.
