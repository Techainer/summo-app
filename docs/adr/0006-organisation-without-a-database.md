# 0006 — Organising notes without a database

**Status:** accepted
**Date:** 2026-08-11
**Extends:** [0002 — no database](0002-no-database.md)

## The decision

Folders, tags and colours are how a vault is organised. All three live in the files themselves —
directory layout for folders, YAML frontmatter for the rest. Summo does not add a database for
organisation, and specifically does not put notes in a hosted one.

```markdown
---
id: 01J4KQ2M3E
date: 2026-08-11T09:00:00+07:00
tags: [sản-phẩm, khách-hàng]
color: "#0f7350"
---

# Weekly product sync
```

## Why the question came up

ADR 0002 settled search. It did not obviously settle *organisation*, and the two feel different: a
tag with a colour, shown in a sidebar, filtered on, reordered by drag — that is the shape of a thing
people reach for a table for. The specific proposal was Turso.

## Why the answer is still no

**0002's measurement covers this too.** The SQLite index was 80 MB for a 65 MB corpus — larger than
the data it indexed — and scanning a thousand meetings takes 30 ms, fast enough to run on every
keystroke. Tags and colours are a few dozen bytes per file. Nothing here is heavier than the thing
already measured and rejected.

**A frontmatter field is not less structured than a column.** It is typed, it is greppable, it
round-trips through the parser that already exists, and `MeetingIndex` already reads frontmatter
when it lists the vault. Adding `tags` and `color` is a struct field, not a subsystem.

**A second store is a second truth.** The vault is the source of truth (0002). A database holding
which folder a note is in means two answers to that question, and they disagree the first time
somebody moves a file in Finder — which is a thing the product actively invites people to do.

**Turso specifically breaks the promise.** Turso is hosted libSQL: SQLite on somebody else's
machine. Notes in it means the notes are on a server, which is the one thing the product says it
does not do. The price only works *because* the user's machine does the heavy part and we hold
nothing; a cloud store of everyone's meeting notes is both a running cost and a liability.

**It would break the sync we have.** `summo-sync` reconciles Markdown files with a three-way merge
and encrypts everything before it leaves. A database beside the vault would need its own sync, its
own conflict rules, and its own encryption — and merging two SQLite files is the worst case there
is. Keeping organisation in the files means organisation syncs for free, the same way agents do.

## Where a database *would* be right

The hosted relay: accounts, billing, device registry, subscription state. That is genuinely
relational, genuinely small, and genuinely nobody's notes. It lives in `summo-cloud`, and Turso is a
reasonable choice for it.

The line is not "no databases anywhere". It is **no database holds a user's content**.

## Consequences

- Tag and colour become `Frontmatter` fields, read by the existing scan.
- Filtering by tag is `MeetingIndex::filter`, not a query.
- A user can add a tag in Obsidian and Summo sees it on the next scan, with no import step.
- A colour is a string in a file the user can edit; there is no palette table to migrate.
- Renaming a tag across a vault is a find-and-replace over Markdown, which `sed` can do and so can
  we. There is no cascade to get wrong.

## What would reopen this

A measurement, not an argument. If a vault large enough to make the scan exceed ~100 ms turns up in
real use — 0002 measured 140 ms at 5,000 meetings, so roughly four times a heavy user's four years —
the answer changes. The benchmark to re-run is `summo-bench vault`, and the number to beat is in
0002.
