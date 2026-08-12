# Translation

Summo translates a meeting into any of 46 languages, **with nothing else installed and nothing
running beside it**. This page is about which model does that and why, because the choice changes
what translation costs, how good it is, and whether any text leaves the machine.

## Two models, not one

| | Summaries, questions, agents | Translation |
|---|---|---|
| Setting | `llm.provider` | `llm.translator` |
| Wants | a model that can read a messy transcript and reason about it | a model that has seen a lot of parallel text |
| Good answer | Qwen3 8B, GPT-5, Claude | MiLMMT-46-1B |
| Where it runs | wherever the user points it | **inside Summo, by default** |
| Prompt | instructions and numbered batches | one line, in the template it was trained on |

They are separate settings because they are separate jobs, and pretending otherwise costs either
money or quality. A **1B** translation model translates better than a general 8B one, runs on a CPU
in about a second a line, and costs nothing per line forever — which is what lets the expensive
model be optional. **A user with no API key at all can still translate every meeting they record.**

If `llm.translator` is absent, translation goes to the general model in numbered batches, exactly as
before. Nothing about an existing setup changes.

## Setting it up

```bash
summo pull milmmt-46-1b     # 806 MB, once — the default
summo pull small100         # or the small one; see "Which model"
```

That is the whole setup. Settings → Translation model → on, and it runs **in the daemon**: no
Ollama, no `llama-server`, no second process to start at login and remember to restart.

Someone who already runs a model server can keep using it — Settings → Runs → *At an endpoint* —
and anything OpenAI-compatible works. That path is one line at a time over HTTP with four in flight;
the in-process path is one line at a time with every thread. They are about the same speed, and the
in-process one needs nothing installed.

The in-process runtimes are behind a build feature:

```bash
cargo build -p summo-cli --release --features serve,transcribe,local-mt
```

That feature turns on both. The ONNX one (`summo-mt/onnx`, for SMALL100) is pure Rust over a
prebuilt `ort`. The GGUF one (`summo-mt/local`, for MiLMMT) compiles llama.cpp, and on Linux that
needs clang's own headers — `libclang-common-18-dev` on Ubuntu — or bindgen fails with
`'stdbool.h' file not found`. A build that only wants the small model can take `summo-mt/onnx`
alone and skip the C++ toolchain entirely.

## Which model

Measured on a Xeon 6226R, eight CPU threads, on Vietnamese meeting speech — dropped pronouns,
English product terms mid-clause, the register a colleague actually uses. Reproduce it with:

```bash
cargo run -p summo-mt --features local --example compare -- model-a.gguf model-b.gguf
```

| Model | Disk | ms/line | Runtime | Verdict |
|---|---|---|---|---|
| **milmmt-46-1b** `Q4_K_M` | 806 MB | ~1150 | llama.cpp | The default |
| milmmt-46-1b `Q3_K_M` | 689 MB | ~620 | llama.cpp | As good, faster, smaller. Worth switching to |
| milmmt-46-1b `Q2_K` | 658 MB | ~707 | llama.cpp | The floor for this model, and slightly worse |
| **small100** int8 | **449 MB** | **~377** | ONNX | The small option. MIT. Lexical errors — see below |
| milmmt-46-4b `Q4_K_M` | 2.5 GB | ~2059 | llama.cpp | Best. Point it at Ollama rather than loading it in-process |
| Qwen3-0.6B `Q4_K_M` | 379 MB | ~1042 | llama.cpp | Collapses |
| Gemma3-270m `Q8_0` | 279 MB | ~172 | llama.cpp | Collapses |

**MiLMMT cannot get much under 650 MB.** It is built on Gemma 3, and a 262 000-token embedding
table dominates the file — `Q2_K` is 658 MB against `Q4_K_M`'s 769 MB, and translates worse. Below
that needs a different architecture, which is what SMALL100 is.

**Small general models are not an option.** Qwen3-0.6B repeated `お疲れ様です` forty times on one
line and returned nothing at all for English→Vietnamese; Gemma3-270m returned empty strings for
three of seven. A model trained for translation beats a general model several times its size.

The 4B fixed every error the 1B made on the sample:

| Vietnamese | 1B | 4B |
|---|---|---|
| chiều nay | 今朝 (this morning) ✗ | 今日の午後 ✓ |
| test tải | 测试下载速度 (download speed) ✗ | 负载测试 ✓ |
| dời mốc ra thứ Sáu | answered in **Thai** ✗ | 締め切りを来週の金曜日に延期します ✓ |

### SMALL100, and a correction

An earlier version of this page rejected [SMALL100](https://huggingface.co/alirezamsh/small100) on
four grounds. Two of them were wrong, and they were wrong in the same way: they were properties of
the *published export*, not of the model.

- ~~1.8 GB~~ — that is the fp32 export. Dynamic int8 quantization takes it to **449 MB**, and the
  output is in the same band. It is the smallest usable translation model here by a wide margin.
- ~~No faster~~ — int8 is **377 ms/line**, the fastest of anything usable.
- **A second runtime** — true, and paid: `summo-mt::seq2seq`. llama.cpp serves decoder-only models,
  so an encoder–decoder needs ONNX Runtime, a SentencePiece BPE segmenter and M2M100's
  language-token scheme. About 700 lines. `ort` was already a dependency for the voice detector, and
  the tokenizer is pure Rust, so unlike the GGUF path this one needs **no C++ toolchain**.
- **Quality** — true, and the real reason to prefer MiLMMT when the disk is there. On Vietnamese
  meeting speech SMALL100 renders "chốt lại spec API" as *lock the spec API*, "dời mốc ra thứ Sáu"
  as *leave the hotel on Friday*, and `go-live` as `GOD`. The sentences are complete, on-topic and
  in the right language; the words are sometimes wrong.

Beam search does not close the gap — the export has no KV cache, so five beams cost five full
forward passes per token (3.3 s/line) and the output was not better. Greedy is used.

It is shipped: `summo pull small100`.

### Getting the 449 MB

The published export is fp32, so `summo pull small100` fetches 1.8 GB. Quantizing is three lines and
about 25 seconds:

```python
from onnxruntime.quantization import quantize_dynamic, QuantType
quantize_dynamic("model.onnx", "model.int8.onnx", weight_type=QuantType.QInt8)
```

Put `model.int8.onnx`, `sentencepiece.bpe.model` and `vocab.json` in a directory and point the
setting at the directory instead of a model id:

```json
"translator": { "provider": "local", "model": "/path/to/small100-int8" }
```

A path is taken as a path before it is tried as a registry id. This is deliberately the exception
rather than the way in — the registry gives content-addressed downloads, resume and a sha256 check,
and none of that applies to a file you made yourself. It exists because the alternative is that the
smallest translation Summo can do is a number in a document.

## Why the prompt is different

MiLMMT, and the NLLB and M2M families before it, were trained to continue exactly one string:

```
Translate this from Vietnamese to Japanese:
Vietnamese: Chiều nay mình chốt lại spec API.
Japanese:
```

Given Summo's numbered-batch prompt instead, a 1B translation model does not translate. Measured:
given three Vietnamese lines and asked for English, it invented a **fourth** line in Vietnamese and
translated none of the three. That is why the model and the prompt are one decision in the code —
`translate::Translator` owns both, so a configuration that pairs the wrong two cannot be expressed.
`prompt::mt_text` is the single source of that string, shared by the HTTP and in-process paths, so
the same line cannot translate differently depending on where the model runs.

## Two things that go wrong, and what catches them

**It samples.** At the shared default temperature of 0.2 a 1B model wanders, so the constructor pins
it to zero — and the in-process runtime decodes greedily with no temperature to set. There is one
right translation of a sentence and no reason to want a different one tomorrow.

**It answers in the wrong language.** Asked for Japanese, MiLMMT-46-1B returned one line of a
three-line meeting in fluent **Thai** — reproducibly, at temperature zero, in both `Q4_K_M` and
`IQ4_XS`, and over both transports. This is a model-level failure, not a quantization artefact and
not a transport bug.

Nothing downstream can see it: a wrong language is not a malformed response. `lang::plausible`
checks the writing system of the reply against the language asked for and drops the line if they
disagree, so the original stays and the run reports itself incomplete. It is deliberately narrow —
Latin text never counts as evidence, because "OK", "API" and every product name on earth appear
verbatim inside correct Japanese.

## Licensing

MiLMMT is published under the **Gemma Terms of Use**, which permit commercial use and impose
conditions on redistribution. Summo therefore does not mirror it: the manifest points upstream and
the user installs it, the same path pyannote and the non-commercial TTS models take. `summo pull`
still does the content-addressed download, the resume and the sha256 check — being unmirrored
changes where the bytes come from, not how carefully they are handled.
