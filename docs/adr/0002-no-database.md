# ADR 0002 — No database. Files and a parallel scan.

**Status:** accepted · **Date:** 2026-08-09 · **Milestone:** M2

## Context

The plan called for SQLite with FTS5 as a derived index over the Markdown vault. The question raised
against it was fair: Obsidian and Claude Code both search by scanning files, and they are not slow.
If a scan is fast enough, a database is a second source of truth that can drift, corrupt, need
migrating, and has to be rebuilt — for nothing.

So we measured it instead of arguing. `summo-bench vault` generates synthetic meetings of realistic
size (~9,000 words each, about an hour of speech) and times both approaches on the same corpus.

## Measurement

```bash
cargo run --release -p summo-bench -- vault --sizes 100,1000,5000
```

| Meetings | Corpus | Scan 1 thread | Scan 8 threads | List (frontmatter) | Index build | Index query | Index size |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 7 MB | 21 ms | **4 ms** | 1 ms | 156 ms | 0.1 ms | 8 MB |
| 1,000 | 65 MB | 209 ms | **30 ms** | 5 ms | 1,600 ms | 0.1 ms | 80 MB |
| 5,000 | 327 MB | 1,033 ms | **140 ms** | 26 ms | 9,310 ms | 0.2 ms | 402 MB |

The parallel column is capped at 8 threads on purpose. This machine has 64; comparing a
single-threaded scan against a database would have rigged the result, and comparing a 64-thread scan
would have rigged it the other way.

Three things fall out:

**The meeting list never needed a database.** Reading each file's first 512 bytes lists 1,000
meetings in 5 ms and 5,000 in 26 ms. That is the operation the app performs on every launch, and it
is free.

**Search is fast enough to be typed into.** 30 ms at 1,000 meetings is inside the window where
results can update per keystroke. 140 ms at 5,000 is fine for search-on-enter. For scale: 1,000
meetings is roughly four years of attending one hour-long meeting every working day.

**The index costs more than the data it indexes.** 80 MB of SQLite for a 65 MB corpus, and 1.6
seconds to build. On a local-first app whose pitch is that your recordings are files on your disk,
shipping a database that is larger than the recordings' text is hard to justify for a saving nobody
can perceive.

## Decision

**No SQLite in v1.** The vault is the only store:

1. **Library listing** — scan frontmatter at startup, keep it in memory. Rebuilt on a file-watcher
   event. At 5,000 meetings this is 26 ms, so there is no cache-invalidation problem to have.
2. **Full-text search** — parallel scan across 8 threads, with the same substring and phrase
   semantics a user expects from grep. No index, no staleness, no rebuild path.
3. **The index that *does* get built later is a vector index**, and only when semantic Q&A ships.
   That is the one capability no amount of scanning provides: "what did we decide about pricing" has
   to match text that never contains the word "pricing". When it arrives it will be an embedding
   store keyed by content hash — deletable, rebuildable, and never consulted for anything a scan can
   answer.

**Revisit when** a real vault exceeds roughly 5,000 meetings, or when search moves to per-keystroke
at that size. The benchmark is committed, so the decision can be re-run rather than re-argued.

## Consequences

- One less subsystem: no schema, no migrations, no "index is corrupt" support burden, no code path
  where the database and the files disagree.
- Deleting a file removes it from search immediately and completely, which is the behaviour users
  expect from a folder and the behaviour a retention policy needs.
- Search semantics are grep's, not FTS5's: no stemming, no ranking by term frequency. For
  transcripts this is arguably better — people search for a phrase they remember hearing — but
  ranked search over a large vault will need a real answer eventually.
- `summo-store` stays in the workspace as an empty crate for the future vector index rather than
  being written now.

## What this does not change

The vault is still the source of truth, and everything derived is still disposable. That principle
was the reason a database was survivable in the first place; measuring it just showed we do not need
one yet.
