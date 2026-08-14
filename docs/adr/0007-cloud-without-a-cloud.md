# 0007 — What a Summo cloud would be, and what it must never become

**Status:** accepted (design; nothing hosted is built)
**Date:** 2026-08-14

## The question

Everything Summo does happens on the machine it runs on. That is the product, and it is why the
transcript of a salary conversation never leaves the laptop. But the questions that keep arriving —
"can my two machines share a vault?", "can my team see the meeting?", "can it use GPT-5 for the
summary?" — are all reasonable, and answering them badly is how a local-first app becomes an
ordinary SaaS with a privacy page.

This is the decision about which of those we will do, in what order, and what the shape has to be
so the first one does not force the rest.

## What already exists

Not a plan on paper — code, with tests:

| | Where | What it does today |
|---|---|---|
| Sync | `summo-sync` | Scan by content hash, three-way merge, conflict copies, sealed blobs |
| Encryption | `summo-sync/src/crypto.rs` | Names and contents encrypted before they leave |
| Remotes | `summo-sync/src/remote.rs` | A `Remote` trait; one implementation, a directory |
| Models | `summo-llm/src/provider.rs` | Ollama, OpenAI, Anthropic, Gemini, Groq, and any base URL |

So "sync to a folder on a NAS, a synced drive or a USB stick" ships today: `summo sync --to`. And a
cloud model is already reachable — it is a setting, not a feature to build.

## Decisions

### 1. A relay stores bytes it cannot read, and nothing else

The `Remote` trait is the whole interface: `get`, `put`, `list`, `manifest`, `salt`. A hosted relay
is one more implementation of it, speaking HTTP instead of the file system. Everything that
understands what a vault *is* — hashing, planning, merging, sealing — stays on the client, where the
key is.

This is the load-bearing decision. A relay that could search transcripts, render a summary or send a
notification about a meeting would be a relay that could read the meeting, and no amount of policy
gets that back. If a feature cannot be built against `get`/`put` of opaque blobs, it does not go in
the relay.

What it still learns: how many objects there are, how big they are, and when they change. That is
metadata and it is not nothing — it is the honest limit of this design, and it belongs in the
documentation rather than in a claim that the relay knows nothing.

### 2. Sync before sharing, and sharing before teams

In order, because each one is a superset of the last:

1. **One person, many machines.** Already the shape `summo-sync` implements. A hosted relay changes
   where the bytes go, not what they are. Key derived from a passphrase the user keeps; lose it and
   the data is gone, said plainly at the moment it is set.
2. **Sharing one meeting.** A single note, sealed to a key in the link — the relay stores a blob and
   never holds the key that opens it. Expiry, and revocation by deleting the blob.
3. **A team vault.** Where it stops being a file-sync problem: two people editing the same note
   within a minute of each other is normal, not an exception, and conflict copies are the wrong
   answer at that rate. This needs a design of its own and is explicitly not in scope here.

### 3. Cloud models are a setting, not a tier

`summo-llm` already speaks to five providers and any OpenAI-compatible base URL. Nothing needs to be
built; what needs to be kept is the property that the default is local, that the screen says which
provider a summary was written by, and that choosing a hosted model is a decision the user makes per
installation rather than a default they discover afterwards.

The same applies in reverse to recognition: sending audio to a hosted recogniser is a bigger
disclosure than sending a summary prompt, and if it is ever offered it needs its own consent, not
inheritance from the model setting.

### 4. No account for the local app

Sync needs somewhere to put bytes, which needs paying for, which needs an account — for the *relay*.
Recording, transcribing, summarising, searching and asking must keep working with no account, no
network and no sign-in, forever. A local-first app that shows a login screen on first run has
already lost the argument it exists to make.

## Consequences

- The relay is small enough to be worth self-hosting, and that should be documented, not
  discouraged: the same protocol, `docker run`, done.
- Server-side search, server-side summarisation and server-side notifications are ruled out by
  decision 1. Anything wanted from them has to happen on a device.
- Sizes and timing leak. Padding is possible later; the honest position now is that it is not done.
- The passphrase is unrecoverable by design. Any future "reset my password" is a redesign, not a
  feature.

## What was rejected

**Signing in with Google to read calendars.** It would mean an OAuth client secret inside an
open-source binary that anyone can read, plus an account-wide scope, so that a notes app can learn
what time the standup is. Summo takes a subscription URL instead — see `summo-engine/src/calsync.rs`
— which grants exactly one calendar and is revocable from the calendar's own settings.

**A hosted "just works" tier where the server holds the key.** It is what most of this category
does, it would be less work, and it would make every other claim in this repository untrue.
