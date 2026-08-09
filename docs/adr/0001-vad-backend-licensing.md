# ADR 0001 — VAD backend: Silero ships, TEN-VAD dropped

**Status:** accepted · **Date:** 2026-08-09 · **Milestone:** M1

## Context

The VAD decides when an utterance ends, and the gate cannot emit final text until it does. Its
release latency is therefore added directly to the delay before a user sees a finished line. TEN-VAD
advertises faster release than Silero, a smaller library, and prebuilt Android/iOS/WASM binaries, so
it was a serious candidate for the default.

We benchmarked both with `summo-bench`, driving the same `Vad` trait the app uses.

## Measurement

30 labelled clips, 262.3 s, 16 kHz mono — the testset published in the TEN-VAD repository. Each
backend is reported at **its own best-F1 threshold**, because the two do not share a probability
calibration and any single fixed threshold flatters whichever one happens to match it.

| Backend | Frame | Threshold | F1 | Precision | Recall | False trigger | Onset p50 | Release p50 | Release p95 | RTF |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **silero v5** (MIT) | 512 (32 ms) | 0.50 | **0.940** | 0.925 | **0.956** | 23.5 % | **17 ms** | 91 ms | 982 ms | 0.0063 |
| silero v4 (MIT) | 512 (32 ms) | 0.40 | 0.898 | 0.904 | 0.892 | 28.7 % | 21 ms | 65 ms | 442 ms | 0.0055 |
| ten-vad | 256 (16 ms) | 0.50 | 0.931 | **0.942** | 0.921 | **17.4 %** | 32 ms | **38 ms** | **362 ms** | 0.0096 |
| ten-vad | 160 (10 ms) | 0.35 | 0.909 | 0.857 | 0.968 | 49.0 % | 10 ms | 109 ms | 857 ms | 0.0098 |

Reproduce:

```bash
SUMMO_TEN_VAD_LIB=/path/to/ten-vad/lib/Linux/x64 \
cargo run --release -p summo-bench --features silero,ten-vad -- vad \
  --dataset /path/to/ten-vad/testset \
  --backend silero:/path/to/silero_vad_v5.onnx \
  --backend silero:/path/to/silero_vad_v4.onnx \
  --backend ten-vad:256 \
  --sweep --json bench/out/vad-sweep.json
```

Read that carefully, because the first run said the opposite.

**Silero v5 is the most accurate backend tested (F1 0.940), and it is MIT.** TEN-VAD is second on
accuracy (0.931) and clearly first on *latency*: release p50 38 ms against Silero v5's 91 ms, and
p95 362 ms against 982 ms. It also has the lowest false-trigger rate.

So the trade is not "faster but unshippable versus slower and legal" — it is **accuracy and licence
versus release latency**. That trade is much easier to accept once you notice the gate already waits
`min_silence_s` = 400 ms before closing a segment: a 91 ms detector release sits inside that window
and is invisible, while the p95 of 982 ms is the number worth attacking later.

### A bug this benchmark caught

Silero v5 initially scored **F1 0.000** — recall zero, every frame silent. The cause was not the
model: the v5 graph expects the caller to prepend 64 samples of the previous frame, so its input is
576 wide even though the hop is 512. Fed a bare 512 samples it does not error; it quietly returns
≈0.001 forever. Without a labelled dataset this would have shipped as "the VAD never triggers on
quiet speakers" and been debugged for days. The v4 path was unaffected, which is exactly why the
loader detects the graph generation instead of assuming one.

Two caveats on the numbers. The dataset is TEN's own published testset, so it is home turf for
TEN-VAD and the comparison should be repeated on our meeting captures. And the 10 ms hop is clearly
worse than the 16 ms hop here, so a smaller hop is not automatically better.

## The blocker

TEN-VAD is **not** Apache-2.0. Its `LICENSE` is Apache-2.0 *plus additional conditions*:

> 1. You may not Deploy the ten-vad in a way that competes with Agora's offerings and/or that allows
>    others to compete with Agora's offerings, including without limitation enabling any third party
>    to develop or deploy Applications.
> 2. You may Deploy the ten-vad solely to create and enable deployment of your Application(s) solely
>    for your benefit and the benefit of your direct End Users.

Three consequences:

1. **Incompatible with our own licence.** GPL-family licences forbid adding restrictions on top of
   the granted rights, so an AGPL-3.0 work cannot contain TEN-VAD. Shipping it would put Summo in
   violation of the licence Summo itself is released under.
2. **Not open source.** The anti-competition clause fails the OSI definition (no field-of-use
   restriction). "Apache-2.0" in a dependency list would be a mislabelling.
3. **Commercially risky, in vague terms.** Agora sells real-time voice infrastructure and a
   conversational-AI engine. Whether a local meeting recorder "competes" is not clearly answerable,
   and the clause also forbids *enabling others* to compete — which is exactly what an open-source
   app with a plugin registry does.

There is also a practical cost: the Linux build links `libc++.so.1`, not `libstdc++`, adding a
runtime dependency most distributions do not install by default.

## Decision

1. **Silero v5 (MIT) is the shipped default.** It is both the most accurate backend measured and the
   only permissively licensed one, and its weaker release latency is absorbed by `VadGate`'s 400 ms
   minimum-silence hysteresis. v4 remains loadable for anyone who already has it.
2. **TEN-VAD is removed from the codebase entirely** — not kept behind a feature flag, not listed in
   the registry. Keeping an unshippable dependency around as an option costs maintenance, invites
   somebody to enable it later without re-reading this document, and leaves a licence question in
   the tree for no benefit now that the permissive option also wins on accuracy. The benchmark that
   produced these numbers is reproducible from the commands above if the comparison ever needs
   revisiting.
3. **`Vad` stays a trait with pluggable backends.** The point of the abstraction survives the
   removal: adding a future detector is one implementation, and the benchmark can rank it against
   Silero the same day.
4. **Benchmark tables keep printing licence and a "shippable" column.** A backend that wins on
   latency but cannot legally ship is not a candidate, and that fact belongs next to the number, not
   in a document nobody re-reads.

*Removed in the commit that followed this decision; the TEN-VAD binding and its manifest are in git
history if the licence ever changes.*

## Follow-ups

- Re-run on our own meeting captures; this dataset is TEN's home turf.
- Silero v5's release p95 of 982 ms is the worst number in the table and would be felt as an
  occasional slow final. Check whether it concentrates in particular clips before deciding it matters.
- A 23.5 % false-trigger rate is high enough to attack directly: try DeepFilterNet ahead of the VAD
  and re-tune `GateConfig::min_speech_s` before concluding the backend is the limit.
- Add a regression test pinning Silero v5's F1 above 0.9, so the 576-vs-512 class of bug cannot
  return silently.
- Find a permissively licensed labelled dataset. Every number above comes from TEN's own testset,
  which is their home turf and now also the testset for a backend we no longer ship.
