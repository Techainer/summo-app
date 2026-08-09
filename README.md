# Summo

Meeting notes that run on your machine. Speech recognition and speaker attribution never leave the
device; only summarisation and translation call out to a language model you configure yourself.

> Status: early. The measurement harness, model registry and VAD layer are implemented and tested.
> Audio capture, ASR sessions and the desktop app are next. See `docs/adr/` for decisions made so far.

## Why it is built this way

The expensive part of a meeting recorder is real-time transcription. Doing it locally removes the
GPU bill entirely, which is what makes it possible to sell the hosted extras cheaply — and it means
the recording of your meeting is a file on your disk rather than a row in someone's database.

Three rules follow from that, and they decide most of the design:

1. **Speech recognition and diarization are always local.** There is no cloud-ASR fallback.
2. **Recording starts in under a second**, with no dialog. Summaries run after the meeting ends,
   because that is when you want them.
3. **Your data is a folder of Markdown files** you can open in Obsidian. The SQLite index is derived
   and can be deleted at any time.

## Layout

```
crates/
  summo-core      shared types: segments, events, paths, errors
  summo-models    Ollama-style model registry: manifests, resumable downloads, blob store, hw probe
  summo-vad       voice activity detection: pluggable backends + the segment gate that drives ASR
  summo-bench     measurement harness — every default should come from a number measured here
  summo-audio     capture (mic + system loopback), resampling, denoise        [in progress]
  summo-asr       decoding sessions: streaming, pseudo-streaming, hybrid       [in progress]
  summo-diar      speaker attribution                                          [in progress]
  summo-vault     Markdown vault                                               [in progress]
  summo-store     SQLite index (FTS5 + vectors)                                [in progress]
  summo-agent     skills, MCP server and client                                [in progress]
  summo-sync      CRDT + end-to-end-encrypted multi-device sync                [in progress]
  summo-engine    local daemon: HTTP + WebSocket on 127.0.0.1                  [in progress]
  summo-cli       `summo pull | list | serve | transcribe`                     [in progress]
registry/         seed model manifests (moves to its own repository)
docs/adr/         decision records
```

## Build and test

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Backends that need a model or a native library are behind Cargo features so the default build works
anywhere:

```bash
# Silero VAD (MIT) — downloads an ONNX Runtime build on first compile
cargo test -p summo-vad --features silero
```

## Benchmarks

Defaults are chosen from measurements, not vibes. To reproduce the VAD comparison:

```bash
curl -L -o /tmp/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx
git clone --depth 1 https://github.com/ten-framework/ten-vad /tmp/ten-vad

cargo run --release -p summo-bench --features silero -- vad \
  --dataset /tmp/ten-vad/testset \
  --backend silero:/tmp/silero_vad.onnx \
  --sweep
```

Results and what they changed: [`docs/adr/0001-vad-backend-licensing.md`](docs/adr/0001-vad-backend-licensing.md).

## Licence

AGPL-3.0-or-later. Models are fetched at runtime and keep their own licences; see `NOTICE`.
