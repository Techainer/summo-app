# Summo

Meeting notes that run on your machine. Speech recognition and speaker attribution never leave the
device; only summarisation and translation call out to a language model you configure yourself.

> Status: the pipeline runs end to end. `summo transcribe` takes a WAV through voice activity
> detection, pseudo-streaming decode and hallucination filtering to a transcript — 2.4 % WER on
> Fleurs VI, RTF 0.11 with live partials. Storage, the local daemon and the desktop app are next.
> Vietnamese and English both transcribe; translation and summaries go through a language model you
> configure. See [`docs/benchmarks.md`](docs/benchmarks.md) for numbers and `docs/adr/` for decisions.

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
  summo-audio     capture (mic), resampling, framing, device selection
  summo-asr       decoding sessions: pseudo-streaming, hybrid refine, sherpa runtime
  summo-vault     the Markdown vault: meetings as files you own
  summo-diar      speaker attribution: track priors, online clustering, refinement
  summo-llm       summaries, translation and Q&A — the only part that leaves the machine
  summo-store     vector index for semantic Q&A                                 [not needed yet]
  summo-agent     skills, MCP server and client                                [in progress]
  summo-sync      CRDT + end-to-end-encrypted multi-device sync                [in progress]
  summo-engine    local daemon: HTTP + WebSocket on 127.0.0.1                  [in progress]
  summo-cli       `summo pull | list | serve | transcribe`                     [in progress]
registry/         seed model manifests (moves to its own repository)
web/              landing page (moves to the cloud repository)
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

To transcribe a recording with the real pipeline:

```bash
# Vietnamese, with a Zipformer transducer
cargo run --release -p summo-cli --features transcribe -- transcribe recording.wav \
  --model-dir /path/to/gipformer-65M --vad /tmp/silero_vad.onnx --threads 4

# English, or any of Whisper's 99 languages
cargo run --release -p summo-cli --features transcribe -- transcribe recording.wav \
  --model-dir /path/to/sherpa-onnx-whisper-tiny --vad /tmp/silero_vad.onnx \
  --engine whisper --lang en
```

To check whether a search index would be worth it on your own corpus size:

```bash
cargo run --release -p summo-bench -- vault --sizes 100,1000,5000
```

## Licence

AGPL-3.0-or-later. Models are fetched at runtime and keep their own licences; see `NOTICE`.
