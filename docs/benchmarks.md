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

**Dataset:** FLEURS `vi_vn` test, **100 clips, 1279.8 s (21.3 min)** — the first 100 by filename, so
the selection is reproducible and not chosen after seeing a score. 16 kHz mono, converted from
FLEURS' float WAVs. Two runs per row, median reported. Xeon Gold 6226R, sherpa-onnx.

| Model | Dataset | Threads | WER | CER | RTF |
|---|---|---:|---:|---:|---:|
| gipformer-65M (int8) | fleurs_vi | 4 | **8.5 %** | 6.7 % | 0.023 |
| gipformer-65M (int8) | fleurs_vi | 8 | 8.6 % | 6.8 % | 0.019 |
| whisper-tiny (fp32) | fleurs_vi | 4 | 67.6 % | 45.1 % | 0.137 |
| whisper-tiny (int8) | fleurs_vi | 4 | 81.3 % | 60.0 % | 0.138 |
| whisper-tiny (fp32) | fleurs_vi | 8 | 67.6 % | 45.1 % | 0.116 |
| whisper-tiny (int8) | fleurs_vi | 8 | 81.0 % | 59.7 % | 0.120 |
| whisper-tiny (fp32) | whisper test set (English) | 4 | **4.5 %** | 0.3 % | 0.107 |

Those rows are the argument for a flat registry rather than a "basic / better / best" ladder.
Whisper-tiny is not a bad model — it scores 4.5 % on English. It is a bad model *for Vietnamese*,
where it is eight times worse than a 73 MB transducer that also runs six times faster. No single
ordering of models is correct across languages, so Summo does not impose one: each manifest states
which languages it was measured on, and the app recommends from that.

The English figure comes from two clips and should be read as "the model works", not as a WER
measurement. A real English number needs LibriSpeech or Common Voice.

**These numbers replace a 15-clip run that reported 2.4 % for gipformer.** 146 s of read speech is
not a measurement, it is an anecdote: it happened to contain no clip with a number in it. Sixteen of
these hundred clips do, and FLEURS writes them as digits (`5 trận`) while every speech model spells
them (`năm trận`), so each one scores as several substitutions. Removing those sixteen clips and
rescoring the remaining 84 gives **5.3 %** for gipformer and 65.7 % for whisper-tiny fp32 — the gap
between 8.5 % and 5.3 % is the dataset's number formatting, not the model. The published figure is
the full hundred, because scoring every model the same way matters more than the flattering subset,
and because a user's meeting will also contain numbers.

The old figure agreed with the Python prototype on the same 15 clips, which was the point of running
it: it confirmed the Rust port feeds the model the same audio and reads back the same text. That
agreement still holds; only the sample it was measured on was too small to publish.

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
16 kHz mono WAVs it names. To rebuild the FLEURS set used above:

```bash
python -c "
from huggingface_hub import hf_hub_download
hf_hub_download('google/fleurs', 'data/vi_vn/audio/test.tar.gz', repo_type='dataset', local_dir='fleurs')
hf_hub_download('google/fleurs', 'data/vi_vn/test.tsv', repo_type='dataset', local_dir='fleurs')"
tar xzf fleurs/data/vi_vn/audio/test.tar.gz -C fleurs
# FLEURS ships float WAVs; hound reads 16-bit PCM.
for f in $(ls fleurs/test | sort | head -100); do sox "fleurs/test/$f" -r 16000 -c 1 -b 16 -e signed-integer "fleurs/wav/$f"; done
# transcripts.json: column 2 of test.tsv is the filename, column 3 the reference.
```

## Quantisation: does int8 pay on a CPU?

The question behind every model we ship: Summo runs on a laptop CPU, and int8 halves the download
and the resident memory. Does it also make inference faster, the way it does on a GPU with INT8
tensor cores?

**On the whole model, no — and for Vietnamese it costs a lot of accuracy.** Same 100-clip FLEURS
set, same runtime, the two builds whisper-tiny publishes:

| | WER | RTF @4t | RTF @8t | Size |
|---|---:|---:|---:|---:|
| fp32 | **67.6 %** | **0.137** | **0.116** | 146 MB |
| int8 | 81.3 % | 0.138 | 0.120 | 99 MB |

Identical speed, **13.7 points worse**. Everything int8 buys here is on disk.

That is not because quantisation does nothing. It is because whisper is an encoder plus an
*autoregressive* decoder, and the two react in opposite directions. Encoder only, one 30 s window,
pinned to four cores, 15 runs, median:

| Runtime | fp32 | int8 | |
|---|---:|---:|---|
| ONNX Runtime 1.28 | 155 ms | **126 ms** | int8 **1.24× faster** |
| OpenVINO 2026.3 | **115 ms** | 182 ms | int8 1.59× *slower* |

So int8 does win where the matrices are large and the pass happens once — the encoder, in the
runtime we ship. It loses in the decoder, which runs a few hundred times per clip on matrices small
enough that the quantise/dequantise around each one costs more than the cheaper multiply saves.
Arithmetic on the numbers above: an average 12.8 s clip costs ~1.75 s at 4 threads, of which the
encoder is ~0.16 s; int8 saves ~0.03 s there and hands it straight back in the decoder. The two
measurements are not pinned the same way — end-to-end runs use the whole machine, the encoder rows
are pinned to four hardware threads — so read the ratios, not the absolute milliseconds.

**And the opposite result, on the model Summo actually defaults to.** `gipformer-65m` publishes an
fp32 build upstream as well; the same 100 clips, four threads:

| gipformer-65m | WER | CER | RTF | Size |
|---|---:|---:|---:|---:|
| int8 | **8.50 %** | 6.73 % | **0.021** | 73 MB |
| fp32 | 8.56 % | **6.43 %** | 0.026 | 250 MB |

Here int8 is **25 % faster**, a third of the size, and its word error rate is the same — the fp32
build's advantage is a quarter of a point of *character* error, which is a slightly better guess at
a wrong word. So the registry ships int8 alone for this model, and there is no fp32 variant to
choose between.

The difference from whisper is the architecture, not the quantisation. A Zipformer transducer spends
almost everything in its encoder, one pass per frame, on matrices large enough for int8 kernels to
pay; its decoder is a single small layer. Whisper spends most of a clip in an autoregressive decoder
that runs hundreds of times on small matrices. Which is why "is int8 faster" has no general answer
and every model gets measured.

The OpenVINO column answers the other suspicion — *maybe this CPU is bad at int8*. This is a
Cascade Lake Xeon with AVX-512 VNNI, the instruction set built for exactly this, and a second
runtime tuned by Intel for Intel reaches the same conclusion by a different route: it is fastest of
all on fp32 and worst of all on this int8 graph. The limit is the shape of the model, not the chip.

**Decision:** where both builds exist, prefer fp32 and fall back to int8 only when memory forces it,
which is what `variant::rank` already does. Publish both — a machine with 2 GB free needs the
smaller one, and 81 % WER is still better than no transcription — but never pick int8 to go faster.
Where only one build is worth shipping, measure before deciding which: for `gipformer-65m` that is
int8, and the fp32 export is not in the registry at all.

**Why not ship OpenVINO, or oneDNN, or MKL:** the ONNX Runtime inside our release exports exactly
one execution provider —

```console
$ nm -D libonnxruntime.so | grep AppendExecutionProvider
000000000036d9c0 T OrtSessionOptionsAppendExecutionProvider_CPU@@VERS_1.17.1
```

— so its kernels are MLAS, which already has AVX-512 and VNNI paths. Adding oneDNN or OpenVINO means
building ONNX Runtime ourselves for five platforms and carrying that build forever, and the fp32
result above (169 ms vs 265 ms) is the whole prize. It is real, and it is not worth a bespoke
toolchain for a decode that is already 6× faster than real time. Revisit if a model ever lands whose
cost is dominated by one big encoder pass.

Graph optimisation is already at maximum everywhere: `summo-mt` and `summo-vad` set
`GraphOptimizationLevel::Level3`, and sherpa-onnx leaves ONNX Runtime's default, which is
`ORT_ENABLE_ALL` — its `SetGraphOptimizationLevel` line is commented out for that reason.

### Reproduce

```bash
# End-to-end, both builds of the same model, on the same dataset.
cargo run --release -p summo-bench --features asr -- asr \
  --dataset fleurs-vi --model whisper:fp32 --model whisper:int8 --lang vi --threads 4

# Encoder only, pinned so the thread pool cannot borrow idle cores.
pip install onnxruntime openvino
taskset -c 0-3 python bench/encoder_precision.py \
  --fp32 fp32/tiny-encoder.onnx --int8 int8/tiny-encoder.int8.onnx
```

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
