# Benchmarks

Every default in Summo should trace back to a number in this file. Results are produced by
`summo-bench`, which drives the same code the app runs.

## Voice activity detection

**Hardware:** Intel Xeon (AVX-512 VNNI), 1 inference thread.
**Dataset:** 30 labelled clips, 262.3 s, 16 kHz mono — the testset published with TEN-VAD.
**Method:** each backend at its own best-F1 threshold, since they do not share a probability
calibration. `--sweep` tries 0.15 … 0.90.

| Backend | Frame | Threshold | F1 | Precision | Recall | False trigger | Onset p50 | Release p50 | Release p95 | RTF | Licence | Shippable |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|:-:|
| **silero v5** | 512 (32 ms) | 0.50 | **0.940** | 0.925 | **0.956** | 23.5 % | **17 ms** | 91 ms | 982 ms | 0.0063 | MIT | yes |
| silero v4 | 512 (32 ms) | 0.40 | 0.898 | 0.904 | 0.892 | 28.7 % | 21 ms | 65 ms | 442 ms | 0.0055 | MIT | yes |
| ten-vad | 256 (16 ms) | 0.50 | 0.931 | **0.942** | 0.921 | **17.4 %** | 32 ms | **38 ms** | **362 ms** | 0.0096 | Apache-2.0 + conditions | **no** |
| ten-vad | 160 (10 ms) | 0.35 | 0.909 | 0.857 | 0.968 | 49.0 % | 10 ms | 109 ms | 857 ms | 0.0098 | Apache-2.0 + conditions | **no** |

**Decision:** ship Silero v5. It is both the most accurate option and the only permissively licensed
one. See [ADR 0001](adr/0001-vad-backend-licensing.md) for the licence analysis and for the
576-vs-512 input bug this benchmark caught.

### What the columns mean

* **Release p50/p95** — delay between speech genuinely stopping and the detector going quiet. The
  gate cannot close a segment until this happens, so it is added directly to the delay before final
  text appears. It is the number most worth optimising.
* **Onset p50** — delay before the detector reacts to speech starting. Largely hidden by the gate's
  400 ms pre-roll buffer.
* **False trigger** — frames called speech during labelled silence, as a fraction of all silence
  frames. This is the "air conditioning keeps opening a segment" number.
* **RTF** — detector compute per second of audio. All candidates are effectively free at ~0.01.

### Reproduce

```bash
curl -L -o /tmp/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx
git clone --depth 1 https://github.com/ten-framework/ten-vad /tmp/ten-vad

cargo run --release -p summo-bench --features silero -- vad \
  --dataset /tmp/ten-vad/testset \
  --backend silero:/tmp/silero_vad.onnx \
  --sweep --json bench/out/vad.json
```

### Caveats

* The dataset is TEN's own published testset — home turf for TEN-VAD. Repeat on our meeting captures
  before treating the ranking as general.
* Single machine, single thread. Numbers are for ranking backends, not for predicting a user's
  laptop; per-machine figures come from the autotune pass at install time.

## Speech recognition — accuracy

Single-pass decode per utterance, scored against reference transcripts. This measures the model;
the session's re-decode multiplier is measured separately below.

**Dataset:** Fleurs VI test, 15 clips, 146.6 s. **Model:** `gipformer-65M` (Zipformer RNN-T, INT8
ONNX, 73 MB) via sherpa-onnx, 4 threads.

| Model | Dataset | Threads | WER | CER | RTF | Audio | Empty |
|---|---|---:|---:|---:|---:|---:|---:|
| gipformer-65M | fleurs_vi | 4 | **2.4 %** | 1.7 % | 0.0240 | 146.6 s | 0 |

**2.4 % WER is exactly what the Python prototype measured on the same model and dataset.** That
agreement is the point of running this: it confirms the Rust port feeds the model the same audio and
reads back the same text, rather than being a plausible-looking reimplementation that quietly
differs. RTF is higher than the prototype's 0.017 because that run used 16 threads against this one's
4, not because the decode changed.

Text is normalised before scoring — lowercased, punctuation stripped, whitespace collapsed. A
transducer emits uppercase without punctuation while the reference has both, and counting that as
substitutions would add several points of error that nobody hears.

### Reproduce

```bash
cargo run --release -p summo-bench --features asr -- asr \
  --dataset /path/to/fleurs_vi \
  --model /path/to/gipformer-65M \
  --threads 4
```

The dataset directory needs a `transcripts.json` of `{wav, text, duration_s}` entries alongside the
16 kHz mono WAVs it names.

## Speech recognition — end-to-end pipeline

First real run of the whole chain: WAV → Silero VAD → `PseudoSession` re-decode loop →
hallucination filter → transcript. Model is `gipformer-65M` (Zipformer RNN-T, INT8 ONNX, 73 MB) via
sherpa-onnx, 4 threads, on the same Xeon.

| Recording | Length | Segments | Decodes | Suppressed | Wall | RTF | Headroom |
|---|---:|---:|---:|---:|---:|---:|---:|
| meeting capture (raw mic) | 34.0 s | 9 | 87 | 0 | 3.62 s | 0.107 | 9× |
| meeting capture (denoised) | 21.7 s | 5 | 79 | 1 | 3.87 s | 0.178 | 6× |

Read the decode counts: 87 decodes for 9 utterances is the pseudo-streaming multiplier — each open
utterance is re-decoded roughly ten times so partial text keeps up with the speaker. **That entire
multiplier costs an RTF of 0.11.** A single pass would be about 0.012, consistent with the 0.017
measured for this model in the Python prototype, so nine tenths of the cost here buys live text and
there is still 9× headroom on this machine.

Sample output (raw capture), reproducing the prototype's transcript:

```
[   4.91s →    5.90s] OK
[   6.19s →    7.78s] BẠN CÓ THỂ NGHE ĐƯỢC KHÔNG
[   8.91s →   10.31s] NGHE TỐT CŨNG KHÔNG
[  12.85s →   15.11s] NẾU MÀ XỬ LÝ TRÊN BỘ ĐỒ NÀY
[  15.57s →   17.45s] THÌ CÓ VẺ LÀ NGON NHỈ
```

### Reproduce

```bash
cargo run --release -p summo-cli --features transcribe -- transcribe recording.wav \
  --model-dir /path/to/gipformer-65M \
  --vad /path/to/silero_vad.onnx \
  --threads 4
```

Add `--partials` to watch text grow inside an utterance rather than only seeing the committed lines.

### Caveats

* Two short captures from one microphone. These numbers say the pipeline works and roughly what it
  costs, not what the accuracy is — no reference transcript was scored, so there is no WER here yet.
* The denoised capture shows a higher RTF because it is shorter, so the fixed cost of the first
  decodes weighs more; per-utterance cost is the same.
* Only one model has been scored so far. The comparison across candidates — Whisper turbo,
  PhoWhisper, SenseVoice, Parakeet — is what decides the shipped default, and needs their runtimes
  wired up first.
* Fleurs is read speech. Meeting audio is harder, and the accuracy gap between the two is the number
  that actually predicts how the app feels.

## Storage: does an index earn its complexity?

Synthetic vault, ~9,000 words per meeting (about an hour of speech). Parallel scan capped at 8
threads so the comparison reflects a laptop rather than this 64-core machine.

| Meetings | Corpus | Scan 1 thread | Scan 8 threads | List (frontmatter) | Index build | Index query | Index size |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 7 MB | 21 ms | **4 ms** | 1 ms | 156 ms | 0.1 ms | 8 MB |
| 1,000 | 65 MB | 209 ms | **30 ms** | 5 ms | 1,600 ms | 0.1 ms | 80 MB |
| 5,000 | 327 MB | 1,033 ms | **140 ms** | 26 ms | 9,310 ms | 0.2 ms | 402 MB |

**Decision: no database.** 30 ms at 1,000 meetings is fast enough to search per keystroke, listing
the library costs 5 ms, and the SQLite index is larger than the corpus it indexes. See
[ADR 0002](adr/0002-no-database.md). The one thing a scan cannot do is semantic search, so a vector
index arrives with Q&A and nothing sooner.

```bash
cargo run --release -p summo-bench -- vault --sizes 100,1000,5000
```
