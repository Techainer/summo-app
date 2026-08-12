# Translation

Summo translates a meeting into any of 46 languages. This page is about **which model does it**,
because that decision changes what translation costs, how good it is, and whether any text leaves
the machine.

## Two models, not one

| | Summaries, questions, agents | Translation |
|---|---|---|
| Setting | `llm.provider` | `llm.translator` |
| Wants | a model that can read a messy transcript and reason about it | a model that has seen a lot of parallel text |
| Good answer | Qwen3 8B, GPT-5, Claude | MiLMMT-46-1B |
| Prompt | instructions and numbered batches | one line, in the template it was trained on |

They are separate settings because they are separate jobs, and pretending otherwise costs either
money or quality. A **1B** translation model translates better than a general 8B one, runs on a CPU
in under a second a line, and costs nothing per line forever — which is what lets the expensive
model be optional. **A user with no API key at all can still translate every meeting they record.**

If `llm.translator` is absent, translation goes to the general model in numbered batches, exactly as
before. Nothing about the existing setup changes.

## Setting it up

Download a GGUF of [MiLMMT-46-1B](https://huggingface.co/xiaomi-research/MiLMMT-46-1B-v1.0) — the
`Q4_K_M` build is about 800 MB — and serve it:

```bash
llama-server -m MiLMMT-46-1B-v1.0.Q4_K_M.gguf --port 8080 -c 4096 -t 8
```

Then turn it on in **Settings → Translation model**, or write it yourself:

```json
{
  "llm": {
    "provider": "ollama",
    "translator": { "provider": "llama-cpp", "model": "milmmt-46-1b" }
  }
}
```

Any OpenAI-compatible server works — llama.cpp, Ollama, LM Studio, vLLM. Summo does not run GGUF
itself, which is deliberate: translation is the one text feature you can have entirely for free, and
that should not depend on Summo shipping its own inference engine.

Measured on a Xeon 6226R, 8 threads, `Q4_K_M`: **about 900 ms a line**, four lines in flight, so an
hour of meeting is a couple of minutes and no money.

## Why the prompt is different

MiLMMT and the NLLB and M2M families before it were trained to continue exactly one string:

```
Translate this from Vietnamese to Japanese:
Vietnamese: Chiều nay mình chốt lại spec API.
Japanese:
```

Given Summo's numbered-batch prompt instead, a 1B translation model does not translate. Measured:
given three Vietnamese lines and asked for English, it invented a **fourth** line in Vietnamese and
translated none of the three. That is why the model and the prompt are one decision in the code —
`translate::Translator` owns both, so a configuration that pairs the wrong two cannot be expressed.

## Two things that go wrong, and what catches them

**It samples.** At the shared default temperature of 0.2, a 1B model wanders. `Translator::mt` pins
it to zero. There is one right translation of a sentence, and no reason to want a different one
tomorrow.

**It answers in the wrong language.** Asked for Japanese, MiLMMT-46-1B returned one line of a
three-line meeting in fluent **Thai** — reproducibly, at temperature zero, in both `Q4_K_M` and
`IQ4_XS`. This is a model-level failure, not a quantization artefact.

Nothing downstream can see that: a wrong language is not a malformed response. `lang::plausible`
checks the writing system of the reply against the language asked for and drops the line if they
disagree, so the original stays and the run reports itself incomplete. It is deliberately narrow —
Latin text never counts as evidence, because "OK", "API" and every product name on earth appear
verbatim inside correct Japanese.

## Choosing a size

| Build | Size | ms/line, 8 threads | Notes |
|---|---|---|---|
| `Q4_K_M` | 806 MB | ~894 | The default recommendation |
| `IQ4_XS` | 718 MB | ~807 | 11% smaller, quality comparable in side-by-side |
| `Q3_K_M` | ~560 MB | — | Untested here |

Both tested builds produced the same wrong-language line, so quantization is not the lever for that.

### Not SMALL100

[SMALL100](https://huggingface.co/alirezamsh/small100) is MIT, 330 MB, and covers 100 languages, so
it looks like the obvious smaller option. It is not, for two reasons:

- **It is a different runtime, not a different file.** SMALL100 is M2M100 — an encoder–decoder
  seq2seq — which llama.cpp does not serve. Using it means writing an ONNX generation loop and a
  SentencePiece tokenizer with SMALL100's own language-token scheme in Rust. That is days of work
  for a model that is worse.
- **It is a 330M model competing with a 1B one at the job the 1B one was built for.** The gap on
  conversational Vietnamese — filler words, dropped pronouns, code-switched English product terms —
  is not close.

If 800 MB is genuinely too much, drop to `IQ4_XS` or `Q3_K_M` before changing model.

## Licensing

MiLMMT is published under the **Gemma Terms of Use**, which permit commercial use and impose
conditions on redistribution. Summo therefore does not mirror it: the manifest points upstream and
the user installs it, the same path pyannote and the non-commercial TTS models take. See
[ADR 0007](adr/) for the general rule about who distributes weights.
