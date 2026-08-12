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
use std::sync::{Arc, Mutex};

use summo_asr::{Decoder, SessionConfig};
use summo_core::{Error, Event, ModelId, Result, segment::Lane};
use summo_diar::{ClusterConfig, OnlineClusterer, embed::SpeakerEmbedder};
use summo_models::{ModelStore, hw::HwProfile};
use summo_pipeline::{
    Frame, Pipeline,
    processors::{Reframe, Tap},
};
use summo_vad::{Vad, silero::SileroVad};

use crate::protocol::SessionSpec;
use crate::stages::{Detect, Recognise};

/// Pull the events out of what a chain produced, dropping the plumbing.
fn events_of(frames: Vec<Frame>) -> Vec<Event> {
    frames
        .into_iter()
        .filter_map(|f| match f {
            Frame::Event(event) => Some(event),
            _ => None,
        })
        .collect()
}

/// One lane's pipeline.
///
/// Assembled from [`summo_pipeline`] stages rather than hand-wired. The chain is
/// `reframe → detect → recognise`, which is what the hand-written loop did in three inlined steps;
/// proven equivalent on real audio before this replaced it — same text, same timestamps, same
/// decode counts, RTF within run-to-run noise. `summo transcribe --pipeline` re-runs that
/// comparison.
///
/// The point of the change is not speed. It is that a stage can now be inserted: a denoiser before
/// the detector, live translation after the recogniser, without editing this struct or anything
/// that builds it.
struct LaneRunner {
    chain: Pipeline,
    /// Audio of the utterance currently being assembled, kept only when this lane is diarized.
    ///
    /// Speaker attribution needs the audio of a *finished* utterance, and the recogniser hands back
    /// text rather than samples, so a tap in the chain keeps a copy here.
    pending: Arc<Mutex<Vec<f32>>>,
}

/// Speaker attribution for the remote lane.
struct Diarizer {
    embedder: SpeakerEmbedder,
    clusterer: OnlineClusterer,
}

/// Drives every lane in a session.
pub struct SessionRunner {
    lanes: HashMap<Lane, LaneRunner>,
    diarizer: Option<Diarizer>,
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
            let width = vad.frame_len();
            let decoder = load_decoder(&spec.live_model, spec.language.as_deref(), store, threads)?;

            let cfg = SessionConfig {
                lane,
                ..SessionConfig::default()
            };

            // Only the system lane is clustered, so only it pays for keeping the audio.
            let pending: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
            let keep = spec.diarize && lane == Lane::System;
            let sink = pending.clone();

            let chain = Pipeline::new()
                .then(Reframe::new(width, summo_core::audio::SAMPLE_RATE))
                // Between the reframer and the detector: the frames here are exactly what the
                // detector will score, which is what makes the kept audio line up with the
                // utterance the recogniser commits.
                .then(Tap::new("keep-audio", move |frame| {
                    if !keep {
                        return;
                    }
                    if let Frame::Audio(audio) = frame
                        && let Ok(mut buffer) = sink.lock()
                    {
                        buffer.extend_from_slice(&audio.samples);
                    }
                }))
                .then(Detect::new(lane, vad))
                .then(Recognise::new(lane, decoder, cfg));

            lanes.insert(lane, LaneRunner { chain, pending });
        }

        // Only the remote lane is clustered. The microphone lane is the local user by construction,
        // so embedding it could only ever discover one speaker at real cost.
        let diarizer = if spec.diarize {
            Some(Diarizer {
                embedder: SpeakerEmbedder::load(resolve_speaker_model(store)?, 1)?,
                clusterer: OnlineClusterer::new(ClusterConfig::default()),
            })
        } else {
            None
        };

        Ok(Self { lanes, diarizer })
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

        let diarize = self.diarizer.is_some() && lane == Lane::System;

        let produced = runner.chain.push(Frame::audio(
            lane,
            samples.to_vec(),
            summo_core::audio::SAMPLE_RATE,
        ))?;
        let mut events = events_of(produced);

        if diarize {
            let audio = runner
                .pending
                .lock()
                .map(|mut buffer| std::mem::take(&mut *buffer))
                .unwrap_or_default();

            let kept = self.attribute(&mut events, &audio);
            // Keep the tail only while an utterance is still open; otherwise the buffer would grow
            // for the whole meeting.
            if let Some(runner) = self.lanes.get_mut(&lane)
                && let Ok(mut buffer) = runner.pending.lock()
            {
                *buffer = kept;
            }
        }
        Ok(events)
    }

    /// Assign a speaker to any final segment in `events`, using the audio that produced it.
    ///
    /// Returns the audio to carry forward: nothing once an utterance has closed, everything when
    /// one is still being assembled.
    fn attribute(&mut self, events: &mut [Event], audio: &[f32]) -> Vec<f32> {
        let Some(diarizer) = self.diarizer.as_mut() else {
            return audio.to_vec();
        };

        let mut closed = false;
        for event in events.iter_mut() {
            let Event::Final(segment) = event else {
                continue;
            };
            closed = true;

            // Embedding failure is not worth losing a transcript line over; the utterance simply
            // goes out unlabelled and the offline pass can attribute it later.
            match diarizer.embedder.embed(audio) {
                Ok(embedding) => {
                    let assignment = diarizer.clusterer.assign(&embedding, segment.duration());
                    if let Some(speaker) = assignment.speaker() {
                        segment.speaker = Some(speaker.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "speaker embedding failed; leaving unlabelled")
                }
            }
        }

        if closed { Vec::new() } else { audio.to_vec() }
    }

    /// Speakers discovered so far in the remote lane.
    #[must_use]
    pub fn speaker_count(&self) -> usize {
        self.diarizer
            .as_ref()
            .map_or(0, |d| d.clusterer.speaker_count())
    }

    /// Close every lane, emitting any utterance still open.
    pub fn flush(&mut self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for runner in self.lanes.values_mut() {
            // `Flush` rather than `End`: a caller may want partial results without tearing the
            // session down, and the pipeline reinstates it for every stage regardless.
            events.extend(events_of(runner.chain.push(Frame::Flush)?));
        }
        Ok(events)
    }

    /// Decode calls made across all lanes, for the performance HUD.
    #[must_use]
    pub fn decode_count(&self) -> u64 {
        self.lanes
            .values()
            .filter_map(|l| l.chain.stage::<Recognise>())
            .map(Recognise::decode_count)
            .sum()
    }

    /// Utterances suppressed as likely hallucinations.
    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        self.lanes
            .values()
            .filter_map(|l| l.chain.stage::<Recognise>())
            .map(Recognise::suppressed_count)
            .sum()
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.lanes
            .values()
            .filter_map(|l| l.chain.stage::<Recognise>())
            .any(Recognise::is_speaking)
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

/// Find an installed speaker-embedding model.
fn resolve_speaker_model(store: &ModelStore) -> Result<std::path::PathBuf> {
    let manifest = store
        .list()
        .into_iter()
        .find(|m| m.task == summo_models::Task::SpeakerEmbed)
        .ok_or_else(|| {
            Error::ModelNotFound(
                "diarization needs a speaker-embedding model. Run `summo pull cam++`.".into(),
            )
        })?;

    let installed = store.resolve(&manifest)?;
    installed
        .param_path("model")
        .or_else(|| installed.files.values().next())
        .cloned()
        .ok_or_else(|| Error::ModelNotFound("the installed speaker model has no model file".into()))
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
    } else if manifest.runtime.contains("sense-voice") {
        let paths = summo_asr::sherpa::SenseVoicePaths {
            model: param_path(&installed, "model")?.display().to_string(),
            tokens: param_path(&installed, "tokens")?.display().to_string(),
        };
        Ok(Box::new(summo_asr::sherpa::SenseVoiceDecoder::load(
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
            gated: false,
            installed_variant: None,
            size_bytes: 0,
            profile: summo_models::Profile::default(),
            files: vec![],
            variants: Vec::new(),
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
