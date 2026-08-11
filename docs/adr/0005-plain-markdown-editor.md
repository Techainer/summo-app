# 0005 — A textarea, not a rich editor

**Status:** accepted
**Date:** 2026-08-11
**Supersedes:** the plan's "Tiptap in `components/notes/Editor.tsx`"

## The decision

Notes are edited in a plain `<textarea>` containing Markdown. Summo does not ship Tiptap,
ProseMirror, Lexical or any other rich-text editor.

This reverses a decision made during planning, so it needs writing down.

## Why the plan said Tiptap

Reasonably. Tiptap is MIT at the core, has a slash-command menu, `@` mentions, tables and task
lists, and every note-taking product people compare Summo to has one. The plan specified it would
"serialise to Markdown so the vault stays the source of truth".

That last clause is where it falls apart.

## What changed

[ADR 0002](0002-no-database.md) commits to the vault being Markdown files a user owns: openable in
Obsidian, greppable, backed up with `cp`, editable by hand. Every feature since has leaned on that
being *literally* true rather than approximately true:

- Tasks are `- [ ] @ngoc … <!-- id:01J4 -->` parsed out of whatever file they are in.
- Unapproved summary sections are marked with an HTML comment, `<!-- summo:draft -->`.
- Transcript lines carry `<!-- seq:7 end:735.20 -->`, which is what lets a translation attach to the
  right utterance.
- The agent reads and rewrites sections of a document a person is also editing.

A rich editor holds a document model of its own and serialises *to* Markdown. That round trip is
lossy for anything the editor does not have a node type for — and every mechanism above is
precisely that: syntax the editor has never heard of, sitting inline in content it does own. The
realistic failure is not a crash. It is a user opening a note, typing one word, and the editor
silently dropping the comment that made a task a task, or reflowing a transcript line so its `seq`
lands on the wrong sentence. Data loss that looks like a successful save.

Making Tiptap safe would mean writing custom nodes for every one of those markers, keeping them in
step with the Rust parsers as both change, and testing the round trip for constructs a user typed
by hand in Obsidian that neither side anticipated. That is a second, worse parser for the vault
format, in a different language.

## What the textarea costs

Honestly: it is less pleasant to write in. No slash menu, no live bold, no drag-to-reorder, no
inline `@` autocomplete. People who want those will notice.

Two things soften it and one does not:

- The first line becomes the title, so there is no form to fill in before writing.
- It autosaves, so the file exists without anybody remembering to press anything.
- Nothing softens the missing formatting toolbar. It is a real gap.

## What would change this

A rich editor whose document model *is* the Markdown — one that treats unknown inline syntax as
opaque text it must preserve byte-for-byte, and can prove it with round-trip tests over the
constructs Summo actually writes. That is a different kind of editor from the ones available today,
and if one appears the trade flips.

The other route is giving up on the vault being hand-editable, which is not on the table: it is the
reason to use Summo instead of a hosted notes app.

## Consequences

- `apps/web/src/screens/NotesScreen.tsx` is one textarea and stays that way.
- Meeting notes get their structure from templates and the agent, not from an editor's UI.
- Anyone wanting a rich editor already has one — Obsidian, on the same files, at the same time.
