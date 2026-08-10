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

### Turso, measured rather than argued

The first version of this ADR dismissed a vector index by reasoning about it. That was not good
enough — indexes are how search normally gets fast, so the objection deserved an experiment.
`summo-bench turso` builds the same voice book in libSQL, creates a DiskANN index with
`libsql_vector_idx()`, and queries it with `vector_top_k()`:

| people | centroids | scan | libSQL scan | **indexed** | **recall@1** | index build | on disk |
|---|---|---|---|---|---|---|---|
| 10 | 80 | 19 µs | 89 µs | 684 µs | 100.0% | 97 ms | 2.7 MB |
| 200 | 1,600 | 302 µs | 953 µs | 5,692 µs | **99.0%** | 7.3 s | 53.1 MB |
| 1,000 | 8,000 | 1,506 µs | 5,847 µs | 7,744 µs | **96.5%** | 37.4 s | 265.5 MB |
| 10,000 | 80,000 | 16,293 µs | 62,230 µs | **8,227 µs** | **74.0%** | 406.6 s | 2,654 MB |

The index is slower than the scan at every size that matters — 36× at ten people, 5× at a thousand.
It finally wins on latency at ten thousand people, by 2×, and that is exactly where the answer stops
being usable: recall 74% means **one voice in four is attributed to the wrong person**. The index
buys 8 ms by getting a quarter of the answers wrong, after a 6.8-minute build and 2.65 GB on disk.

Recall is measured against the exact scan, so these are not "slightly worse matches" — they are
different people. At a thousand people, 96.5% is already one utterance in twenty-nine.

This is not a defect in Turso. It is what an ANN index *is*: an approximation that trades exactness
for asymptotics, and the trade only pays once N is large enough to cover the constant factor. The
measurement locates that crossover precisely — it is at ten thousand people, and by then the
approximation has degraded to the point where the speed is worthless. A voice book is bounded by how
many people one user actually meets; ten thousand of them is not a scale Summo needs to serve, and
it is the *first* size where the index is even faster.

The recall column is the decisive one. Elsewhere an approximate index costs a slightly worse search
result. Here it puts the wrong name on a sentence somebody said, in a document the user will send to
their team.

Sync is Turso's other draw, and it is real — but Summo's sync must be end-to-end encrypted with the
vault as the source of truth (see `docs/sync-protocol.md`). A hosted database that can read the rows
is the architecture this product exists to avoid. If embeddings sync one day they travel as
ciphertext like everything else.

*(Reproduce: `cargo run --release -p summo-bench --no-default-features --features turso -- turso --people 10,200,1000,10000`.
The feature is off by default because libsql and rusqlite each bundle a SQLite amalgamation and
cannot link together.)*

### Vectors are only comparable within one model's space

An embedding means nothing on its own — only next to other embeddings from the *same* model. The
trap is that CAM++ and ERes2NetV2 both emit **192 dimensions**, so a dimension check passes while
the vectors describe unrelated coordinate systems. Similarities would scatter around zero, Summo
would quietly stop recognising people it had been taught, and would occasionally score a stranger
high by chance.

So a book records the *identity* of the model that produced its vectors, not just their width
(`summo_diar::space::EmbeddingSpace`: model id, revision, dims). `VoiceBook::adopt` is called before
any comparison and refuses a mismatch instead of approximating through it. Recovery is
`summo_diar::space::plan`: re-embed from the audio if retention kept it, otherwise
`reset_vectors`, which drops the vectors but keeps every person, name and avatar. Losing recognition
is recoverable — the user's voices are relearned as they speak. Silent misattribution is not.

### What stays a file for a different reason

Transcripts, notes and exports (SRT, VTT, Markdown) stay human-readable files under ADR 0002. That
is not a performance decision and this measurement does not touch it.

## Consequences

* `VoiceLog` gains a binary encoding; the JSON reader stays for one release so existing vaults are
  read, then migrated on write.
* The vectors remain deletable. Removing `~/.summo/voices/` costs recognition, never a transcript.
* The header records the embedding space — model, revision and dimension — so a mismatch is detected
  rather than silently compared. Dimension alone is not enough; see above.
* Identification stays a linear scan. If a future Summo ever holds enough vectors for that to hurt,
  the fix is to scan fewer of them (compare against the centroids of plausible people first), not to
  accept an approximate answer.
