# 3. Speaker embeddings stay in files, in a binary format

Date: 2026-08-10

## Status

Accepted.

## Context

ADR 0002 decided transcripts and notes are Markdown files, because a user has to be able to grep,
edit, sync and back them up. Speaker embeddings are a different kind of data and the argument does
not transfer: nobody reads a 192-dimensional vector, and the only operation that matters is
comparing all of them against a handful of centroids when somebody names a voice and the whole
history has to be relabelled.

So the question was asked properly: should embeddings live in a database — SQLite, or Turso/libSQL
for its sync and vector search?

## Measurement

`summo-bench voices`, on this machine, one full resweep — load every vector and score it against
80 centroids (about ten people at eight centroids each):

| vectors | format | on disk | write | full sweep |
|---|---|---|---|---|
| 20,000 (200 meetings) | json | 42.1 MB | 135 ms | 454 ms |
| | **binary** | **14.6 MB** | **18 ms** | **310 ms** |
| | sqlite | 31.9 MB | 65 ms | 335 ms |
| 200,000 (1,000 meetings) | json | 421.4 MB | 1339 ms | 4569 ms |
| | **binary** | **146.5 MB** | **317 ms** | **3077 ms** |
| | sqlite | 318.9 MB | 651 ms | 3170 ms |

## Decision

**No database. Binary vector files per meeting, under `~/.summo/voices/meetings/`.**

The measurement says something more useful than which option won. At 200,000 vectors the sweep is
3.07 GFLOP of dot products, and all three formats land within a few hundred milliseconds of each
other around three seconds — because the arithmetic is the floor and every format pays it. SQLite's
contribution is not making the comparison faster; it is making the *loading* faster than JSON, which
a binary file also does, for a third of the disk and none of the dependency.

JSON was the actual mistake. It cost 2.9× the disk and 1.5 s of parsing at 200,000 vectors, for a
payload with no human reader.

Two consequences follow:

* A **full** resweep is only needed when the whole book changes. Naming one person needs their
  centroids compared, not everyone's — an 80× reduction that puts a typical correction in the tens
  of milliseconds. The benchmark measures the pessimistic case on purpose.
* 200,000 utterances is several years of heavy use. At the scale a real user reaches this year, a
  correction is imperceptible whatever the storage.

### Why not Turso

Turso/libSQL offers two things worth wanting: sync, and ANN vector search. Neither applies.

The ANN index answers "find the nearest among millions". Summo asks "score these against eighty",
which a linear scan already does at the FLOP floor — an index would add work, not remove it. And
sync is real, but Summo's sync has to be end-to-end encrypted with the vault as the source of truth
(see `docs/sync-protocol.md`); a hosted database that can read the rows is the architecture this
product exists to avoid. If embeddings sync one day they will travel as ciphertext like everything
else.

### What stays a file for a different reason

Transcripts, notes and exports (SRT, VTT, Markdown) stay human-readable files under ADR 0002. That
is not a performance decision and this measurement does not touch it.

## Consequences

* `VoiceLog` gains a binary encoding; the JSON reader stays for one release so existing vaults are
  read, then migrated on write.
* The vectors remain deletable. Removing `~/.summo/voices/` costs recognition, never a transcript.
* If a future embedding model changes dimension, the header records it, so a mismatch is detected
  rather than silently compared.
