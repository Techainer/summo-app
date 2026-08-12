//! Translation models running in this process.
//!
//! Everything else in Summo that touches a language model talks HTTP to something the user
//! started — Ollama, llama.cpp, LM Studio, or a hosted API. That is the right default for
//! summaries, where the model is large and the choice is genuinely the user's. It is the wrong
//! default for **translation**, because translation is the one text feature that can be free: a
//! small translation model runs on a CPU in well under a second a line and costs nothing per line
//! forever.
//!
//! Requiring a second program to be installed and running is what stops that being true in
//! practice. "Install Ollama, pull a model, keep it running" is three steps between a user and a
//! feature that costs nothing — and on a machine that already has Summo, all three are avoidable.
//!
//! ## Two runtimes, because there are two shapes of translation model
//!
//! | | [`gguf`] | [`seq2seq`] |
//! |---|---|---|
//! | Runs | MiLMMT and other decoder-only models | SMALL100 and the M2M100 family |
//! | Through | llama.cpp | ONNX Runtime |
//! | Smallest useful | 658 MB | **449 MB** |
//! | Speed | ~620 ms/line | ~377 ms/line |
//! | Quality | better | lexical errors, right language |
//! | Build cost | a C++ toolchain | none beyond `ort` |
//!
//! The second exists because the first cannot get small. MiLMMT is built on Gemma 3, and a
//! 262 000-token embedding table dominates the file — quantizing to `Q2_K` gets to 658 MB and
//! translates worse than `Q4_K_M` at 769 MB. Below that needs a different architecture, and
//! llama.cpp does not serve encoder–decoder models, so it needs a different runtime too.
//!
//! Both were measured against every small *general* model that could plausibly compete, and those
//! collapsed: Qwen3-0.6B repeated one phrase forty times and returned nothing for
//! English→Vietnamese; Gemma3-270m returned empty strings for three of seven lines. A model trained
//! for translation beats a general model several times its size, which is the finding this whole
//! module rests on.
//!
//! ## What this is not
//!
//! Not a translator. These are *runtimes*: text in, text out. The translation template lives in
//! `summo-llm` beside the one the HTTP path uses, so the two cannot drift into prompting the same
//! model differently.
//!
//! Not a replacement for the HTTP path. Someone already running Ollama should keep using it, and
//! someone who wants a hosted model must be able to have one. These are the options that need
//! nothing installed.

#[cfg(feature = "local")]
pub mod gguf;
#[cfg(feature = "onnx")]
pub mod seq2seq;
#[cfg(feature = "onnx")]
pub mod spm;

#[cfg(feature = "local")]
pub use gguf::Local;
#[cfg(feature = "onnx")]
pub use seq2seq::{Seq2Seq, Seq2SeqPaths};
