//! Load an installed model and run it once, to find out whether it actually works.
//!
//! Nothing in this product could answer that question. Installing checks a sha256, which proves the
//! bytes arrived intact and nothing else: a manifest can name a `params` key that no file matches,
//! a variant can resolve to a build that was never downloaded, an archive can unpack into a
//! directory the runtime cannot open, and a build can lack the runtime entirely. Every one of those
//! has happened here, and every one surfaced the same way — at the start of a recording, or at the
//! first translation, minutes or weeks after the install that caused it, with the error naming a
//! hashed path in a shard directory.
//!
//! So: load it the way the product loads it, and run one inference. Not a benchmark and not an
//! accuracy measurement — the question is binary and the answer is worth a few seconds of CPU.
//!
//! **The same code path, not a copy of it.** Every arm below calls the function the daemon calls:
//! [`crate::runner::load_decoder`] for speech, [`crate::translate::check_local`] for translation.
//! A verifier with its own loading rules verifies its own loading rules.
//!
//! What a pass means is narrow and worth stating: the runtime accepted the files and returned
//! without error. It does not mean the transcript will be good. A model that mishears every word
//! passes this, and should — "is it working" and "is it accurate" are different questions, and
//! `docs/benchmarks.md` is where the second one is answered.

use std::time::Instant;

use summo_models::{Manifest, ModelStore, Task};

/// One model's result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Check {
    pub id: String,
    /// `serde` spelling, matching everything else the catalogue sends.
    pub task: Task,
    /// Whether the model loaded and ran.
    pub ok: bool,
    /// What happened, in a sentence: what it produced, or why it did not.
    pub detail: String,
    /// Wall clock for load plus one inference, milliseconds.
    ///
    /// Not a benchmark — a cold load dominates it — but the number that tells somebody whose app
    /// "hangs when recording starts" that the model takes 40 seconds to open.
    pub millis: u128,
}

/// One second of a quiet 220 Hz tone at 16 kHz.
///
/// Silence would be simpler and is a worse probe: a speaker embedder refuses empty audio, some
/// graphs short-circuit on an all-zero input, and a recogniser fed pure silence exercises less of
/// itself than one fed a signal. A tone is deterministic, costs nothing to generate, and is not
/// speech — which is fine, because no arm here asserts on *what* came back.
#[cfg(feature = "models")]
fn probe() -> Vec<f32> {
    let rate = summo_core::audio::SAMPLE_RATE as f32;
    (0..summo_core::audio::SAMPLE_RATE as usize)
        .map(|i| {
            let t = i as f32 / rate;
            0.2 * (std::f32::consts::TAU * 220.0 * t).sin()
        })
        .collect()
}

/// Check every installed model.
///
/// Sequential on purpose. These load hundreds of megabytes into ONNX sessions, and running them
/// concurrently on a laptop is how a check turns into a swap storm on the machine it was meant to
/// reassure.
pub fn check_all(store: &ModelStore, threads: usize) -> Vec<Check> {
    store
        .list()
        .iter()
        .map(|manifest| check(store, manifest, threads))
        .collect()
}

/// Check one.
///
/// Never returns an error: a model that fails is the *result*, not a failure of the checking. A
/// caller wants the row either way, and a `Result` here would mean the first broken model hides
/// every model after it.
#[must_use]
pub fn check(store: &ModelStore, manifest: &Manifest, threads: usize) -> Check {
    let started = Instant::now();
    let id = manifest.id.to_string();

    // Before anything is loaded: does this binary have the runtime at all. Reported as a failure
    // rather than skipped, because from where the user stands an installed model that cannot run is
    // broken, whichever half of the reason it is.
    let outcome = match crate::runtimes::why_not(&manifest.runtime) {
        Some(why) => Err(why),
        None => run(store, manifest, threads),
    };

    let (ok, detail) = match outcome {
        Ok(detail) => (true, detail),
        Err(why) => (false, why),
    };

    Check {
        id,
        task: manifest.task,
        ok,
        detail,
        millis: started.elapsed().as_millis(),
    }
}

/// Load and run, per task. `Err` carries the sentence shown to the user.
fn run(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    match manifest.task {
        Task::Asr => asr(store, manifest, threads),
        Task::Vad => vad(store, manifest, threads),
        Task::Denoise => denoise(store, manifest, threads),
        Task::SpeakerEmbed => speaker(store, manifest, threads),
        Task::Translate => translate(store, manifest, threads),
        Task::Tts => tts(store, manifest, threads),
        // Two task names with no model in the registry and no feature behind them. Saying so beats
        // both a false pass and a false failure.
        Task::DiarizeSeg | Task::Embed => {
            Err("Summo has no runtime for this kind of model yet".into())
        }
    }
}

/// The path a recording takes, on one second of tone.
#[cfg(feature = "models")]
fn asr(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    use summo_asr::Decoder as _;

    let mut decoder = crate::runner::load_decoder(manifest.id.as_str(), None, store, threads)
        .map_err(|e| e.to_string())?;
    let transcript = decoder.decode(&probe()).map_err(|e| e.to_string())?;

    // Deliberately not "it heard nothing, so it is broken". A transducer given a tone correctly
    // returns nothing, and a Whisper model given the same tone often invents a sentence. Both are
    // working. What is reported is what happened, so a reader can tell the two apart.
    Ok(match transcript.text.trim() {
        "" => "loaded and decoded one second of audio (no words, as expected from a tone)".into(),
        text => format!("loaded and decoded one second of audio: {:?}", clip(text)),
    })
}

#[cfg(feature = "models")]
fn vad(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    use summo_vad::Vad as _;

    let path = file(store, manifest, "model")?;
    let mut vad = summo_vad::silero::SileroVad::load(&path, threads).map_err(|e| e.to_string())?;
    let frame = vad.frame_len();
    let audio = probe();
    let probability = vad
        .feed_frame(&audio[..frame.min(audio.len())])
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "loaded and scored one {frame}-sample frame ({probability:.2} speech)"
    ))
}

#[cfg(feature = "models")]
fn denoise(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    use summo_asr::denoise::Denoiser as _;

    let path = file(store, manifest, "model")?;
    let mut model = summo_asr::denoise::Gtcrn::load(
        &path.display().to_string(),
        u32::try_from(threads).unwrap_or(1),
        manifest.id.as_str(),
    )
    .map_err(|e| e.to_string())?;
    let cleaned = model.denoise(&probe()).map_err(|e| e.to_string())?;
    Ok(format!(
        "loaded and cleaned one second of audio ({} samples in, {} out)",
        summo_core::audio::SAMPLE_RATE,
        cleaned.len()
    ))
}

#[cfg(feature = "models")]
fn speaker(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    let path = file(store, manifest, "model")?;
    let mut embedder =
        summo_diar::embed::SpeakerEmbedder::load(&path, threads).map_err(|e| e.to_string())?;
    let vector = embedder.embed(&probe()).map_err(|e| e.to_string())?;
    Ok(format!(
        "loaded and embedded one second of audio ({} dimensions)",
        vector.len()
    ))
}

/// One short line through the real translator.
///
/// English into Vietnamese because both are in every translation manifest the registry publishes,
/// so the check does not depend on the user's language being one the model covers.
#[cfg(feature = "mt-any")]
fn translate(store: &ModelStore, manifest: &Manifest, _threads: usize) -> Result<String, String> {
    let line = "The meeting starts at nine.";
    match crate::translate::check_local(store, manifest.id.as_str(), line, "vi", None)
        .map_err(|e| e.to_string())?
    {
        // Loading succeeded and the model produced nothing for an ordinary sentence. That is a
        // failure, and one no digest can see: it is what a mismatched vocabulary or a missing
        // sentencepiece model looks like from outside.
        None => Err("loaded, but translated an ordinary sentence into nothing".into()),
        Some(text) => Ok(format!(
            "loaded and translated {line:?} to {:?}",
            clip(&text)
        )),
    }
}

/// One word through the voice.
///
/// The check a voice needs more than any other model: it arrives as a 397-member archive, and
/// everything that can go wrong with unpacking one — a missing `espeak-ng-data` table, a `params`
/// key pointing at the wrong level of the directory — produces a voice that is present, correct by
/// digest, and silent.
#[cfg(feature = "tts")]
fn tts(store: &ModelStore, manifest: &Manifest, threads: usize) -> Result<String, String> {
    let installed = store.resolve(manifest).map_err(|e| e.to_string())?;
    let dir = installed.param_dir("dir").ok_or_else(|| {
        format!(
            "`{}` has no `params.dir` naming an unpacked directory",
            manifest.id
        )
    })?;

    let mut voice = summo_tts::vits::Vits::load(&dir, threads).map_err(|e| e.to_string())?;
    let speech = voice.say_at("Xin chào", 1.0).map_err(|e| e.to_string())?;
    if speech.samples.is_empty() {
        return Err("loaded, but synthesised no audio for a short line".into());
    }
    Ok(format!(
        "loaded and spoke one line ({} samples at {} Hz)",
        speech.samples.len(),
        speech.rate
    ))
}

/// Resolve one `params` key to a path on disk, in the words a reader needs.
#[cfg(feature = "models")]
fn file(store: &ModelStore, manifest: &Manifest, key: &str) -> Result<std::path::PathBuf, String> {
    let installed = store.resolve(manifest).map_err(|e| e.to_string())?;
    installed
        .param_path(key)
        .or_else(|| installed.files.values().next())
        .cloned()
        .ok_or_else(|| {
            format!(
                "`{}` has no `params.{key}` naming an installed file",
                manifest.id
            )
        })
}

/// Keep a model's output to a length that fits on a card.
#[cfg(any(feature = "models", feature = "mt-any", test))]
fn clip(text: &str) -> String {
    const MAX: usize = 60;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX).collect::<String>() + "…"
}

// The arms this build has no runtime for. They are unreachable — `check` asks `runtimes::why_not`
// first and every one of these tasks answers with a reason — but a `match` still has to be total,
// and a `todo!()` here would be a panic waiting for the one cfg combination nobody built.
#[cfg(not(feature = "models"))]
fn asr(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no speech recognition in it".into())
}
#[cfg(not(feature = "models"))]
fn vad(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no voice detection in it".into())
}
#[cfg(not(feature = "models"))]
fn denoise(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no noise suppression in it".into())
}
#[cfg(not(feature = "models"))]
fn speaker(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no speaker embedding in it".into())
}
#[cfg(not(feature = "mt-any"))]
fn translate(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no translation runtime in it".into())
}
#[cfg(not(feature = "tts"))]
fn tts(_: &ModelStore, _: &Manifest, _: usize) -> Result<String, String> {
    Err("this build has no speech synthesis in it".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ModelStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(summo_core::paths::Paths::at(tmp.path()));
        (tmp, store)
    }

    fn manifest(id: &str, task: &str, runtime: &str) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "schema": 1,
            "id": id,
            "name": id,
            "task": task,
            "mode": "batch",
            "runtime": runtime,
            "langs": ["*"],
            "license": "MIT",
            "size_bytes": 1,
            "files": [],
            "params": {},
        }))
        .unwrap()
    }

    /// A model whose runtime this build lacks fails before anything is loaded, and says which.
    ///
    /// `mt-gguf` is off in `cargo test` and in every shipped binary, so the GGUF translator is the
    /// real instance of this rather than an invented one — it is the model that spent every release
    /// downloadable and unloadable.
    #[cfg(not(feature = "mt-gguf"))]
    #[test]
    fn a_runtime_this_build_lacks_fails_without_touching_the_disk() {
        let (_tmp, store) = store();
        // Deliberately not installed. Reaching the store at all would be the bug: the answer is
        // known from the manifest, and a check that needs the blobs cannot run before a download.
        let check = check(
            &store,
            &manifest("milmmt-46-4b", "translate", "llama.cpp/gguf"),
            1,
        );
        assert!(!check.ok, "a missing runtime reported a pass");
        assert!(check.detail.contains("mt-gguf"), "{}", check.detail);
        assert!(
            check.detail.contains("choose another model"),
            "{}",
            check.detail
        );
    }

    /// A runtime nobody here has heard of is a failure, not a crash and not a pass.
    #[test]
    fn an_unknown_runtime_is_reported_rather_than_guessed_at() {
        let (_tmp, store) = store();
        let check = check(&store, &manifest("future", "asr", "tensorrt/whatever"), 1);
        assert!(!check.ok);
        assert!(
            check.detail.contains("newer than this version"),
            "{}",
            check.detail
        );
    }

    /// The two task names with no model behind them say so instead of failing obscurely.
    ///
    /// Goes at `run` rather than `check`, deliberately. `check` asks the runtime table first, and
    /// in a build without a given feature *every* manifest is refused there — so through `check`
    /// this would pass in `cargo test` for the wrong reason and stop testing the thing it names the
    /// moment somebody enabled a feature. `run` is the function that holds the arm.
    #[test]
    fn a_task_with_no_runtime_anywhere_says_that_rather_than_erroring_on_files() {
        let (_tmp, store) = store();
        for task in ["embed", "diarize-seg"] {
            let why = run(&store, &manifest("someday", task, "sherpa-onnx/whisper"), 1)
                .expect_err("a task with nothing behind it reported a pass");
            assert!(why.contains("no runtime for this kind"), "{why}");
        }
    }

    #[test]
    fn output_is_clipped_so_a_talkative_model_cannot_break_a_card() {
        let long = "a".repeat(200);
        let clipped = clip(&long);
        assert!(clipped.chars().count() <= 61, "{clipped}");
        assert!(clipped.ends_with('…'));
        assert_eq!(clip("  short  "), "short");
    }

    /// An empty store checks nothing and does not fail doing it.
    #[test]
    fn nothing_installed_is_an_empty_list() {
        let (_tmp, store) = store();
        assert!(check_all(&store, 1).is_empty());
    }
}
