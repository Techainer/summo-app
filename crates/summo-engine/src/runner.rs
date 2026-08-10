//! Turning received audio into transcript events.
//!
//! Without this the daemon is a socket that counts samples. This is where the frames arriving over
//! the wire actually meet the voice detector and the decoder.
//!
//! Two details do real work. Incoming frames are whatever size the client chose, while the VAD
//! wants an exact width, so a [`Framer`] sits between them — a mismatch here would silently corrupt
//! the detector's recurrent state. And each lane gets its own runner: two speakers on one gate
//! would interleave into a single garbled utterance, and lane separation is the cheapest speaker
//! attribution Summo has.

use std::collections::HashMap;

use summo_asr::{Decoder, PseudoSession, SessionConfig};
use summo_audio::Framer;
use summo_core::{Error, Event, ModelId, Result, segment::Lane};
use summo_models::{ModelStore, hw::HwProfile};
use summo_vad::{Vad, silero::SileroVad};

use crate::protocol::SessionSpec;

/// One lane's pipeline.
struct LaneRunner {
    vad: Box<dyn Vad>,
    framer: Framer,
    session: PseudoSession<Box<dyn Decoder>>,
}

/// Drives every lane in a session.
pub struct SessionRunner {
    lanes: HashMap<Lane, LaneRunner>,
}

impl SessionRunner {
    /// Load the models a session needs and prepare a pipeline per lane.
    ///
    /// Models are loaded once and cloned per lane would be wrong — a decoder holds mutable
    /// inference state — so each lane loads its own. That doubles resident memory for a two-lane
    /// session, which is the price of transcribing both sides of a call independently.
    pub fn new(spec: &SessionSpec, store: &ModelStore, hw: &HwProfile) -> Result<Self> {
        spec.validate()?;

        let vad_model = resolve_vad(store)?;
        let threads = hw.recommended_threads();

        let mut lanes = HashMap::new();
        for &lane in &spec.lanes {
            let vad: Box<dyn Vad> = Box::new(SileroVad::load(&vad_model, 1)?);
            let decoder = load_decoder(&spec.live_model, spec.language.as_deref(), store, threads)?;

            let cfg = SessionConfig {
                lane,
                ..SessionConfig::default()
            };
            lanes.insert(
                lane,
                LaneRunner {
                    framer: Framer::new(vad.frame_len()),
                    vad,
                    session: PseudoSession::new(decoder, cfg),
                },
            );
        }

        Ok(Self { lanes })
    }

    /// Feed audio for one lane, returning whatever it produced.
    pub fn accept(&mut self, lane: Lane, samples: &[f32]) -> Result<Vec<Event>> {
        let Some(runner) = self.lanes.get_mut(&lane) else {
            // A client sending audio for a lane it did not ask for is a bug worth surfacing rather
            // than silently transcribing.
            return Err(Error::Other(format!(
                "received audio for lane `{}`, which this session did not open",
                lane.as_str()
            )));
        };

        // Collect the exact-width frames first: the closure cannot borrow `runner` mutably while
        // `framer` is already borrowed from it.
        let mut frames: Vec<Vec<f32>> = Vec::new();
        runner
            .framer
            .push(samples, |frame| frames.push(frame.to_vec()));

        let mut events = Vec::new();
        for frame in frames {
            let prob = runner.vad.feed_frame(&frame)?;
            events.extend(runner.session.accept(&frame, prob)?);
        }
        Ok(events)
    }

    /// Close every lane, emitting any utterance still open.
    pub fn flush(&mut self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for runner in self.lanes.values_mut() {
            events.extend(runner.session.flush()?);
        }
        Ok(events)
    }

    /// Decode calls made across all lanes, for the performance HUD.
    #[must_use]
    pub fn decode_count(&self) -> u64 {
        self.lanes.values().map(|l| l.session.decode_count()).sum()
    }

    /// Utterances suppressed as likely hallucinations.
    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        self.lanes
            .values()
            .map(|l| l.session.suppressed_count())
            .sum()
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.lanes.values().any(|l| l.session.is_speaking())
    }
}

/// Find an installed voice detector.
fn resolve_vad(store: &ModelStore) -> Result<std::path::PathBuf> {
    let manifest = store
        .list()
        .into_iter()
        .find(|m| m.task == summo_models::Task::Vad)
        .ok_or_else(|| {
            Error::ModelNotFound(
                "no voice detector installed. Run `summo pull silero-vad-v5`.".into(),
            )
        })?;

    let installed = store.resolve(&manifest)?;
    installed
        .param_path("model")
        .or_else(|| installed.files.values().next())
        .cloned()
        .ok_or_else(|| {
            Error::ModelNotFound("the installed voice detector has no model file".into())
        })
}

/// Resolve a `params` key to a concrete blob path, naming what is missing when it is not there.
///
/// This indirection is not incidental. The blob store is content-addressed, so an installed file is
/// named after its hash and sits in a shard directory beside unrelated blobs. Nothing can be found
/// by looking for `encoder.onnx` on disk — the manifest's `params` are the only mapping from a
/// runtime's expectations to real paths.
fn param_path(
    installed: &summo_models::store::InstalledModel,
    key: &str,
) -> Result<std::path::PathBuf> {
    installed
        .param_path(key)
        .cloned()
        .ok_or_else(|| Error::InvalidManifest {
            id: installed.manifest.id.to_string(),
            reason: format!("no `params.{key}` naming an installed file"),
        })
}

/// Load the speech model named by the session, choosing a runtime from its manifest.
fn load_decoder(
    id: &str,
    language: Option<&str>,
    store: &ModelStore,
    threads: usize,
) -> Result<Box<dyn Decoder>> {
    let model_id = ModelId::parse(id).map_err(Error::Config)?;
    let manifest = store.installed(&model_id)?;
    let installed = store.resolve(&manifest)?;

    if manifest.runtime.contains("whisper") {
        let paths = summo_asr::sherpa::WhisperPaths {
            encoder: param_path(&installed, "encoder")?.display().to_string(),
            decoder: param_path(&installed, "decoder")?.display().to_string(),
            tokens: param_path(&installed, "tokens")?.display().to_string(),
        };
        Ok(Box::new(summo_asr::sherpa::WhisperDecoder::load(
            &paths, language, threads, id,
        )?))
    } else if manifest.runtime.contains("transducer") {
        let paths = summo_asr::sherpa::TransducerPaths {
            encoder: param_path(&installed, "encoder")?.display().to_string(),
            decoder: param_path(&installed, "decoder")?.display().to_string(),
            joiner: param_path(&installed, "joiner")?.display().to_string(),
            tokens: param_path(&installed, "tokens")?.display().to_string(),
        };
        Ok(Box::new(summo_asr::sherpa::ZipformerDecoder::load(
            &paths, threads, id,
        )?))
    } else {
        Err(Error::UnsupportedRuntime(manifest.runtime.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::paths::Paths;

    fn store() -> (tempfile::TempDir, ModelStore) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        (tmp, ModelStore::new(paths))
    }

    #[test]
    fn a_session_without_an_installed_vad_says_what_to_install() {
        let (_tmp, store) = store();
        let Err(err) = SessionRunner::new(&SessionSpec::new("m"), &store, &HwProfile::detect())
        else {
            panic!("a session without an installed VAD should not start")
        };
        let err = err.to_string();
        assert!(
            err.contains("summo pull"),
            "the error should be actionable: {err}"
        );
    }

    /// The bug this guards: `params` are the only way to find a file in a content-addressed store,
    /// so a manifest missing one must fail with a message naming the key rather than with a
    /// confusing "file not found" for a path nobody wrote.
    #[test]
    fn a_manifest_missing_a_params_entry_names_the_key() {
        use summo_models::store::InstalledModel;

        let manifest = summo_models::Manifest {
            schema: 1,
            id: ModelId::parse("broken").unwrap(),
            name: "Broken".into(),
            task: summo_models::Task::Asr,
            mode: summo_models::Mode::Batch,
            runtime: "sherpa-onnx/transducer-offline".into(),
            langs: vec![],
            domains: vec![],
            license: "MIT".into(),
            attribution: None,
            redistributable: true,
            size_bytes: 0,
            profile: summo_models::Profile::default(),
            files: vec![],
            params: Default::default(),
            description: None,
        };
        let installed = InstalledModel {
            manifest,
            files: Default::default(),
        };

        let err = param_path(&installed, "encoder").unwrap_err().to_string();
        assert!(err.contains("params.encoder"), "got: {err}");
    }

    #[test]
    fn an_invalid_spec_is_refused_before_any_model_loads() {
        let (_tmp, store) = store();
        let mut spec = SessionSpec::new("m");
        spec.diarize = true; // without the system lane

        let Err(err) = SessionRunner::new(&spec, &store, &HwProfile::detect()) else {
            panic!("an invalid spec should not start a session")
        };
        assert!(err.to_string().contains("diarization"), "got: {err}");
    }
}
