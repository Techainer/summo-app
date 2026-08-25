//! Which model runtimes this binary contains, and which manifest belongs to which.
//!
//! Two questions that have to have the same answer and did not.
//!
//! Loading a model matched its manifest's `runtime` string with a chain of `contains` — one chain
//! in [`crate::runner`] for recognition, another in [`crate::translate`] for translation — and
//! nothing anywhere asked the *first* question: does this build have that runtime at all. The
//! catalogue therefore offered every model in the registry to every build.
//!
//! That is not hypothetical. The release ships `mt-onnx` and not `mt-gguf`, because llama.cpp needs
//! a C++ toolchain at build time, so the two MiLMMT models are 0.8 GB and 2.4 GB that no released
//! binary has ever been able to load. The screen offered them, the download worked, the digest
//! matched, and the failure arrived at the first translation as "this build has no runtime for a
//! GGUF file" — a sentence about a compile-time flag, shown to somebody who has just spent 2.4 GB
//! of a Vietnamese home connection.
//!
//! [`crate::onboarding`] already learned this lesson once, for recognition:
//!
//! > a property of the binary, not of the machine […] setup offered the catalogue, downloaded
//! > 99 MB of Whisper, and then could not use it.
//!
//! It fixed it for one feature with one `cfg!`. This is that answer for every runtime, in one
//! place, so the next feature added does not get to relearn it.
//!
//! The other half is drift. Two independent `contains` chains meant a runtime could be dispatchable
//! and unadvertised, or advertised and undispatchable, with nothing to notice. Both loaders now go
//! through [`kind_of`], so "which runtime runs this" and "do we have it" are answered by the same
//! table or not at all.

/// A runtime this codebase knows how to drive.
///
/// Not the manifest's string: that is a publisher's spelling, and several spellings map to one
/// implementation. This is the implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Whisper through sherpa-onnx: encoder, decoder, tokens.
    Whisper,
    /// SenseVoice through sherpa-onnx: one graph plus tokens.
    SenseVoice,
    /// A Zipformer RNN-T through sherpa-onnx: encoder, decoder, joiner, tokens.
    Transducer,
    /// Silero voice activity detection.
    SileroVad,
    /// GTCRN speech enhancement.
    Gtcrn,
    /// A speaker-embedding model through sherpa-onnx.
    SpeakerEmbed,
    /// A VITS voice: a directory, not a file.
    Vits,
    /// An M2M100-family sequence-to-sequence translator through ONNX Runtime.
    Seq2Seq,
    /// A decoder-only translator through llama.cpp.
    Gguf,
}

impl Kind {
    /// The feature that supplies this runtime, in the words a `--features` flag wants.
    ///
    /// For an error message, and only for one: a user cannot enable a feature in a binary somebody
    /// else built, so the sentence this appears in has to offer a different model as well.
    #[must_use]
    pub fn feature(self) -> &'static str {
        match self {
            Self::Whisper
            | Self::SenseVoice
            | Self::Transducer
            | Self::SileroVad
            | Self::Gtcrn
            | Self::SpeakerEmbed => "models",
            Self::Vits => "tts",
            Self::Seq2Seq => "mt-onnx",
            Self::Gguf => "mt-gguf",
        }
    }
}

/// Which runtime a manifest's `runtime` string names, if this codebase drives one.
///
/// Substring matching rather than equality, which is what both loaders already did: a publisher
/// writes `sherpa-onnx/transducer-offline` and a later one may write `sherpa-onnx/transducer-v2`,
/// and neither is a new implementation.
///
/// **Order is load-bearing.** `sherpa-onnx/silero-vad` and `onnx/m2m100` both contain `onnx`, so
/// the specific names are tested first and the bare `onnx` fallback last — that fallback exists
/// because [`crate::translate`] has always matched translation models with `contains("onnx")`, and
/// narrowing it here to `m2m100` would quietly stop a registry from publishing a different seq2seq
/// export. A model is only offered this fallback after its task has been checked, so a speech model
/// cannot fall into it.
#[must_use]
pub fn kind_of(runtime: &str) -> Option<Kind> {
    if runtime.contains("whisper") {
        Some(Kind::Whisper)
    } else if runtime.contains("sense-voice") {
        Some(Kind::SenseVoice)
    } else if runtime.contains("transducer") {
        Some(Kind::Transducer)
    } else if runtime.contains("silero") {
        Some(Kind::SileroVad)
    } else if runtime.contains("gtcrn") {
        Some(Kind::Gtcrn)
    } else if runtime.contains("speaker-embedding") {
        Some(Kind::SpeakerEmbed)
    } else if runtime.contains("vits") {
        Some(Kind::Vits)
    } else if runtime.contains("gguf") {
        Some(Kind::Gguf)
    } else if runtime.contains("onnx") {
        Some(Kind::Seq2Seq)
    } else {
        None
    }
}

/// Whether this binary was built with the runtime that drives `kind`.
#[must_use]
pub fn have(kind: Kind) -> bool {
    match kind {
        Kind::Whisper
        | Kind::SenseVoice
        | Kind::Transducer
        | Kind::SileroVad
        | Kind::Gtcrn
        | Kind::SpeakerEmbed => cfg!(feature = "models"),
        Kind::Vits => cfg!(feature = "tts"),
        Kind::Seq2Seq => cfg!(feature = "mt-onnx"),
        Kind::Gguf => cfg!(feature = "mt-gguf"),
    }
}

/// Whether this binary can load the model a manifest describes.
///
/// An unknown runtime answers `false`: a registry may publish something newer than the app reading
/// it, and "we do not know what this is" and "we cannot run it" are the same fact to a user
/// deciding whether to spend the download.
#[must_use]
pub fn runnable(runtime: &str) -> bool {
    kind_of(runtime).is_some_and(have)
}

/// Why a model cannot be loaded here, or `None` when it can.
///
/// Phrased for the person holding a binary rather than a checkout: naming the missing feature alone
/// would be an instruction they cannot follow, so it always ends with the thing they *can* do.
#[must_use]
pub fn why_not(runtime: &str) -> Option<String> {
    match kind_of(runtime) {
        None => Some(format!(
            "this app has no runtime for `{runtime}`; it may be newer than this version of Summo"
        )),
        Some(kind) if !have(kind) => Some(format!(
            "this build of Summo has no runtime for `{runtime}`; choose another model, or build \
             with the `{}` feature",
            kind.feature()
        )),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every runtime string the registry publishes today, mapped.
    ///
    /// Hard-coded rather than read from the registry directory: this asserts what the *app* must
    /// know, and a checkout without the registry beside it would otherwise skip the whole thing.
    #[test]
    fn every_published_runtime_is_recognised() {
        for (runtime, expected) in [
            ("sherpa-onnx/whisper", Kind::Whisper),
            ("sherpa-onnx/sense-voice", Kind::SenseVoice),
            ("sherpa-onnx/transducer-offline", Kind::Transducer),
            ("onnx/silero-vad", Kind::SileroVad),
            ("sherpa-onnx/gtcrn", Kind::Gtcrn),
            ("sherpa-onnx/speaker-embedding", Kind::SpeakerEmbed),
            ("sherpa-onnx/vits", Kind::Vits),
            ("onnx/m2m100", Kind::Seq2Seq),
            ("llama.cpp/gguf", Kind::Gguf),
        ] {
            assert_eq!(kind_of(runtime), Some(expected), "for `{runtime}`");
        }
    }

    /// The trap the ordering exists for.
    ///
    /// Six of the nine strings above contain `onnx`, and the translation fallback is the last
    /// branch. Written as its own test because the failure it prevents — every sherpa model
    /// classified as a translator — is silent: `kind_of` still returns `Some`, the catalogue still
    /// says runnable, and the wrong loader is chosen at the moment a recording starts.
    #[test]
    fn the_onnx_fallback_does_not_swallow_the_sherpa_runtimes() {
        for runtime in [
            "sherpa-onnx/whisper",
            "sherpa-onnx/sense-voice",
            "sherpa-onnx/transducer-offline",
            "onnx/silero-vad",
            "sherpa-onnx/gtcrn",
            "sherpa-onnx/speaker-embedding",
            "sherpa-onnx/vits",
        ] {
            assert_ne!(
                kind_of(runtime),
                Some(Kind::Seq2Seq),
                "`{runtime}` fell through to the translation fallback"
            );
        }
    }

    #[test]
    fn an_unknown_runtime_is_not_runnable_and_says_why() {
        assert_eq!(kind_of("tensorrt/something"), None);
        assert!(!runnable("tensorrt/something"));
        let why = why_not("tensorrt/something").expect("an unknown runtime has a reason");
        assert!(why.contains("newer than this version"), "{why}");
    }

    /// The one this module was written for: a runtime the codebase knows and this build lacks.
    ///
    /// `mt-gguf` is off in every shipped binary and in `cargo test`, so this is the real case
    /// rather than a contrived one.
    #[cfg(not(feature = "mt-gguf"))]
    #[test]
    fn a_gguf_translator_is_refused_before_the_download() {
        assert!(!runnable("llama.cpp/gguf"));
        let why = why_not("llama.cpp/gguf").expect("a missing runtime has a reason");
        assert!(why.contains("mt-gguf"), "{why}");
        assert!(
            why.contains("choose another model"),
            "the reader cannot rebuild somebody else's binary: {why}"
        );
    }

    /// And the same runtime in a build that has it.
    #[cfg(feature = "mt-gguf")]
    #[test]
    fn a_gguf_translator_is_fine_in_a_build_with_llama_cpp() {
        assert!(runnable("llama.cpp/gguf"));
        assert_eq!(why_not("llama.cpp/gguf"), None);
    }
}
