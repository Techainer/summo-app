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

## Speech recognition

Not yet measured. Candidate list and the metrics to collect are in the implementation plan; the
harness gains a `summo-bench asr` subcommand alongside the ASR runtimes.
