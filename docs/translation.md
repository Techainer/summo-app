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
summo pull milmmt-46-1b     # 806 MB, once
```

That is the whole setup. Settings → Translation model → on, and it runs **in the daemon**: no
Ollama, no `llama-server`, no second process to start at login and remember to restart.

Someone who already runs a model server can keep using it — Settings → Runs → *At an endpoint* —
and anything OpenAI-compatible works. That path is one line at a time over HTTP with four in flight;
the in-process path is one line at a time with every thread. They are about the same speed, and the
in-process one needs nothing installed.

The in-process runtime is behind a build feature, because it compiles llama.cpp:

```bash
cargo build -p summo-cli --release --features serve,transcribe,local-mt
```

On Linux that build needs clang's own headers — `libclang-common-18-dev` on Ubuntu — or bindgen
fails with `'stdbool.h' file not found`.

## Which model

Measured on a Xeon 6226R, eight CPU threads, on Vietnamese meeting speech — dropped pronouns,
English product terms mid-clause, the register a colleague actually uses. Reproduce it with:

```bash
cargo run -p summo-mt --features local --example compare -- model-a.gguf model-b.gguf
```

| Model | Disk | ms/line | Verdict |
|---|---|---|---|
| **milmmt-46-1b** | 806 MB | ~1150 | The default. Fits everywhere, good enough for meeting notes |
| **milmmt-46-4b** | 2.5 GB | ~2059 | Better on every sentence tried. Worth it if the memory is there |
| SMALL100 (ONNX) | 1.8 GB | ~785 | Worse on every sentence tried. Not shipped — see below |

The 4B fixed every error the 1B made on the sample:

| Vietnamese | 1B | 4B |
|---|---|---|
| chiều nay | 今朝 (this morning) ✗ | 今日の午後 ✓ |
| test tải | 测试下载速度 (download speed) ✗ | 负载测试 ✓ |
| dời mốc ra thứ Sáu | answered in **Thai** ✗ | 締め切りを来週の金曜日に延期します ✓ |

### Not SMALL100

[SMALL100](https://huggingface.co/alirezamsh/small100) is MIT, 330M parameters and covers 100
languages, so it looks like the obvious smaller option. It was tried properly — real ONNX inference,
real SentencePiece tokenizer, the same sentences — and it lost on every axis that matters:

- **Quality.** "chốt lại spec API" became *lock up the spec API*; the Japanese school sentence came
  back as Vietnamese that does not mean what the original said; the line the 1B got wrong in Thai,
  SMALL100 got wrong in Japanese.
- **Size.** The published ONNX export is **1.8 GB** — more than twice the 1B GGUF. Dynamic int8
  would bring it to about 450 MB, at a further quality cost that was not worth measuring given the
  starting point.
- **Speed.** ~785 ms/line, no better, because the export carries no KV cache: the 12-layer encoder
  re-runs for *every generated token*.
- **Cost to ship.** llama.cpp does not serve encoder–decoder seq2seq, so it would mean a second
  runtime — an ONNX generation loop plus SMALL100's own language-token scheme — in Rust.

**If 800 MB is too much, drop the quantization, not the model.** `IQ4_XS` is 718 MB at ~807 ms/line
with comparable output; `Q3_K_M` is about 560 MB.

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
