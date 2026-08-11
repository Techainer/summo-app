//! A real synthesiser, through sherpa-onnx's VITS runtime.
//!
//! This is the backend the crate was written around a trait for. It runs through **the same native
//! library the recogniser already links**, which is why it is a feature rather than a dependency:
//! nothing new is pulled in, and a build that already does speech recognition can do speech
//! synthesis for the cost of a flag.
//!
//! ## Why VITS and not VieNeu-TTS
//!
//! VieNeu-TTS is Apache-2.0 and is the better model. It is also a two-stage LM-plus-codec system
//! with no runtime in this dependency tree — wiring it means implementing that pipeline against
//! ONNX Runtime directly and verifying it against weights, which is a project rather than a commit.
//!
//! VITS is one ONNX graph, sherpa already drives it, and permissively-licensed Vietnamese and
//! English voices exist for it. So dubbing works today with a model a user can `summo pull`,
//! and [`crate::Synthesizer`] is the seam VieNeu-TTS slots into when somebody does that work. The
//! planning and mixing halves — the parts with the hard decisions in them — do not change either
//! way; they were written against the trait for exactly this reason.
//!
//! ## Speed
//!
//! `length_scale` stretches or compresses at synthesis time, which is strictly better than
//! resampling afterwards: the model produces speech at the target duration rather than speech that
//! has been sped up, so there is no pitch shift. [`crate::dub::stretch`] stays for the residual —
//! the model does not hit a requested duration exactly — but the bulk of the fitting happens here.

use std::path::Path;

use summo_core::{Error, Result};

// Only the backend uses these; the path resolution above is deliberately buildable without a
// native library, so a machine with no ONNX can still test how a voice directory is read.
#[cfg(feature = "sherpa")]
use crate::{Speech, Synthesizer, Voice};

/// Where a VITS voice's files are.
///
/// `tokens` and `model` are required; the rest are optional and model-specific. A lexicon is how a
/// Vietnamese model spells out numbers and loanwords, and omitting one that a model expects
/// produces speech that skips them silently rather than failing.
#[derive(Debug, Clone)]
pub struct VitsFiles {
    pub model: std::path::PathBuf,
    pub tokens: std::path::PathBuf,
    pub lexicon: Option<std::path::PathBuf>,
    pub dict_dir: Option<std::path::PathBuf>,
    pub data_dir: Option<std::path::PathBuf>,
}

impl VitsFiles {
    /// Resolve from a directory laid out the way sherpa's released models are.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let model = first_matching(dir, |name| name.ends_with(".onnx")).ok_or_else(|| {
            Error::msg(
                "tts.no_model",
                format!("{} có không .onnx nào", dir.display()),
            )
        })?;
        let tokens = dir.join("tokens.txt");
        if !tokens.is_file() {
            return Err(Error::msg(
                "tts.no_tokens",
                format!("{} thiếu tokens.txt", dir.display()),
            ));
        }

        Ok(Self {
            model,
            tokens,
            lexicon: Some(dir.join("lexicon.txt")).filter(|p| p.is_file()),
            dict_dir: Some(dir.join("dict")).filter(|p| p.is_dir()),
            data_dir: Some(dir.join("espeak-ng-data")).filter(|p| p.is_dir()),
        })
    }
}

fn first_matching(dir: &Path, matches: impl Fn(&str) -> bool) -> Option<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&matches))
        .collect();
    // Sorted so a directory with two graphs picks the same one every run rather than whichever the
    // filesystem happened to list first.
    found.sort();
    found.into_iter().next()
}

/// A VITS voice.
#[cfg(feature = "sherpa")]
pub struct Vits {
    tts: sherpa_rs::tts::VitsTts,
    rate: u32,
}

#[cfg(feature = "sherpa")]
impl Vits {
    /// Load a voice from a directory of model files.
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let files = VitsFiles::from_dir(dir)?;
        let path =
            |p: Option<std::path::PathBuf>| p.map(|p| p.display().to_string()).unwrap_or_default();

        let config = sherpa_rs::tts::VitsTtsConfig {
            model: files.model.display().to_string(),
            tokens: files.tokens.display().to_string(),
            lexicon: path(files.lexicon),
            dict_dir: path(files.dict_dir),
            data_dir: path(files.data_dir),
            // The model's own defaults. Overriding these without measuring is how synthesis starts
            // sounding subtly wrong in a way nobody can attribute.
            length_scale: 1.0,
            noise_scale: 0.667,
            noise_scale_w: 0.8,
            silence_scale: 0.2,
            onnx_config: sherpa_rs::OnnxConfig {
                num_threads: threads as i32,
                ..Default::default()
            },
            ..Default::default()
        };

        Ok(Self {
            tts: sherpa_rs::tts::VitsTts::new(config),
            // Filled in from the first synthesis: the model states its own rate, and guessing here
            // would mean every dubbed track resampled by a factor nobody chose.
            rate: 0,
        })
    }

    /// Synthesise at a chosen speed.
    ///
    /// `speed` above 1.0 is faster. Done in the model rather than by resampling afterwards, so a
    /// line fitted into a shorter slot keeps its pitch — see the note at the top of this module.
    pub fn say_at(&mut self, text: &str, speed: f32) -> Result<Speech> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Speech {
                samples: Vec::new(),
                rate: self.rate.max(1),
            });
        }

        let audio = self
            .tts
            // Speaker 0: a single-speaker model has only one, and a multi-speaker one needs a
            // choice the caller has not been given a way to make yet.
            .create(text, 0, speed.max(0.1))
            .map_err(|e| Error::msg("tts.failed", format!("không tổng hợp được: {e}")))?;

        self.rate = audio.sample_rate;
        Ok(Speech {
            samples: audio.samples,
            rate: audio.sample_rate,
        })
    }
}

#[cfg(feature = "sherpa")]
impl Synthesizer for Vits {
    fn rate(&self) -> u32 {
        // Before the first synthesis the model has not said. 22.05 kHz is what the VITS voices
        // sherpa publishes use, and the real value replaces it the moment anything is spoken.
        if self.rate == 0 { 22_050 } else { self.rate }
    }

    fn say(&mut self, text: &str, _voice: &Voice) -> Result<Speech> {
        self.say_at(text, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let path = tmp.path().join(name);
            if name.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::write(&path, body).unwrap();
            }
        }
        tmp
    }

    #[test]
    fn a_model_directory_resolves_its_graph_and_tokens() {
        let dir = dir_with(&[("vi.onnx", "x"), ("tokens.txt", "a")]);
        let files = VitsFiles::from_dir(dir.path()).unwrap();
        assert!(files.model.ends_with("vi.onnx"));
        assert!(files.tokens.ends_with("tokens.txt"));
    }

    /// Omitting a lexicon a model expects produces speech that silently skips numbers and
    /// loanwords, so the absence is recorded rather than assumed away.
    #[test]
    fn optional_files_are_absent_rather_than_empty_paths() {
        let dir = dir_with(&[("vi.onnx", "x"), ("tokens.txt", "a")]);
        let files = VitsFiles::from_dir(dir.path()).unwrap();
        assert!(files.lexicon.is_none());
        assert!(files.dict_dir.is_none());
    }

    #[test]
    fn a_lexicon_and_dictionary_are_found_when_they_are_there() {
        let dir = dir_with(&[
            ("vi.onnx", "x"),
            ("tokens.txt", "a"),
            ("lexicon.txt", "b"),
            ("dict/", ""),
        ]);
        let files = VitsFiles::from_dir(dir.path()).unwrap();
        assert!(files.lexicon.is_some());
        assert!(files.dict_dir.is_some());
    }

    #[test]
    fn a_directory_with_no_graph_says_so() {
        let dir = dir_with(&[("tokens.txt", "a")]);
        let err = VitsFiles::from_dir(dir.path()).unwrap_err().to_string();
        assert!(err.contains(".onnx"), "{err}");
    }

    #[test]
    fn a_directory_with_no_tokens_says_so() {
        let dir = dir_with(&[("vi.onnx", "x")]);
        let err = VitsFiles::from_dir(dir.path()).unwrap_err().to_string();
        assert!(err.contains("tokens.txt"), "{err}");
    }

    /// A directory with two graphs must resolve the same one every run, not whichever the
    /// filesystem listed first.
    #[test]
    fn resolution_is_stable_when_there_is_more_than_one_graph() {
        let dir = dir_with(&[("b.onnx", "x"), ("a.onnx", "x"), ("tokens.txt", "t")]);
        let first = VitsFiles::from_dir(dir.path()).unwrap().model;
        let again = VitsFiles::from_dir(dir.path()).unwrap().model;
        assert_eq!(first, again);
        assert!(first.ends_with("a.onnx"));
    }
}
