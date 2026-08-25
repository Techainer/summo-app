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

use std::collections::{HashMap, VecDeque};
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
    /// Finished utterances waiting for the refine model, filled by every refining lane.
    ///
    /// One queue for the whole session rather than one per lane, because there is one refine
    /// decoder: a decoder holds mutable inference state, so two lanes cannot run it at once, and a
    /// queue per lane would only be two orderings of the same single-file work.
    refine_queue: Arc<Mutex<VecDeque<summo_asr::RefineJob>>>,
}

impl SessionRunner {
    /// Load the models a session needs and prepare a pipeline per lane.
    ///
    /// Models are loaded once and cloned per lane would be wrong — a decoder holds mutable
    /// inference state — so each lane loads its own. That doubles resident memory for a two-lane
    /// session, which is the price of transcribing both sides of a call independently.
    pub fn new(spec: &SessionSpec, store: &ModelStore, hw: &HwProfile) -> Result<Self> {
        Self::with_warm(spec, store, hw, None)
    }

    /// The same, but allowed to take an already-built decoder out of the warm slot.
    ///
    /// Only the first lane can use it — a decoder holds mutable inference state, so two lanes
    /// cannot share one — which is why this is a parameter rather than something `new` reaches for
    /// on its own: the caller owns the slot and decides.
    pub fn with_warm(
        spec: &SessionSpec,
        store: &ModelStore,
        hw: &HwProfile,
        warm: Option<&crate::warm::Warm>,
    ) -> Result<Self> {
        spec.validate()?;

        let vad_model = resolve_vad(store, spec.vad_model.as_deref())?;
        let threads = hw.recommended_threads();

        // Before the lanes, like the refine model and for the same reason: a named enhancer that
        // cannot be found should fail the recording here, with a message about the thing that was
        // asked for, rather than after the first sentence.
        let denoise_model = resolve_denoise_model(store, spec.denoise_model.as_deref())?;

        let key = crate::warm::Key::new(&spec.live_model, spec.language.clone(), threads);
        // Taken, not borrowed: whoever gets it owns it, and the slot is refilled afterwards. That
        // keeps every question about a killed recording holding a borrowed decoder from existing.
        let mut ready = warm.and_then(|warm| warm.take(&key));

        // A second model, when one is named and it is not the one already decoding. `validate`
        // refuses the identical pair, which would decode everything twice for nothing.
        //
        // Loaded before the lanes, so a refine model that is missing fails the session here rather
        // than after the first utterance — and does not fail it at all when it is simply unset,
        // which is what everybody who has not asked for this has.
        let refine_queue: Arc<Mutex<VecDeque<summo_asr::RefineJob>>> =
            Arc::new(Mutex::new(VecDeque::new()));
        let refining = spec.refine_model.is_some();

        let mut lanes = HashMap::new();
        for &lane in &spec.lanes {
            let vad: Box<dyn Vad> = Box::new(SileroVad::load(&vad_model, 1)?);
            let width = vad.frame_len();
            let decoder = match ready.take() {
                Some(decoder) => decoder,
                None => load_decoder(&spec.live_model, spec.language.as_deref(), store, threads)?,
            };

            let cfg = SessionConfig {
                lane,
                ..SessionConfig::default()
            };

            // One per lane. The enhancer carries inference state across the frames of a call the
            // same way a decoder does, so two lanes sharing one would clean the microphone with
            // state left over from the room — and cost a mutex on the finalize path to do it.
            let denoiser: Option<Box<dyn summo_asr::Denoiser>> = match &denoise_model {
                None => None,
                // One thread, and not `threads`. Measured on the real export: 1 thread runs a three
                // second utterance at real-time factor 0.102, 2 at 0.115, 4 at 0.114, 8 at 0.120 —
                // every extra thread makes it *slower*. GTCRN is small and causal, so there is
                // little to parallelise and the synchronisation costs more than it saves. Handing
                // it the machine's recommended thread count would take cores off the decoder to run
                // the enhancer worse.
                Some(path) => Some(Box::new(summo_asr::denoise::Gtcrn::load(
                    &path.display().to_string(),
                    1,
                    spec.denoise_model.as_deref().unwrap_or("denoiser"),
                )?)),
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
                .then(
                    if refining {
                        Recognise::refining(lane, decoder, cfg, refine_queue.clone())
                    } else {
                        Recognise::new(lane, decoder, cfg)
                    }
                    .with_denoiser(denoiser),
                );

            lanes.insert(lane, LaneRunner { chain, pending });
        }

        // Only the remote lane is clustered. The microphone lane is the local user by construction,
        // so embedding it could only ever discover one speaker at real cost.
        // Asked for, and skipped when the model for it is not installed.
        //
        // `?` here refused the entire session: turning on system audio sets `diarize`, diarization
        // needs a speaker-embedding model nobody has installed, and the whole recording failed —
        // with a message about a model the user never asked for. Telling apart two voices is a
        // nice-to-have on top of a recording; a recording is not a nice-to-have on top of it.
        let diarizer = match spec
            .diarize
            .then(|| resolve_speaker_model(store, spec.speaker_model.as_deref()))
        {
            Some(Ok(model)) => Some(Diarizer {
                embedder: SpeakerEmbedder::load(model, 1)?,
                clusterer: OnlineClusterer::new(ClusterConfig::default()),
            }),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "recording without speaker attribution");
                None
            }
            None => None,
        };

        Ok(Self {
            lanes,
            diarizer,
            refine_queue,
        })
    }

    /// Take the utterances waiting to be decoded again by the slower model.
    ///
    /// Drained by the caller rather than handled here, because running one is a second of blocking
    /// work and this type is called from the frame loop. Empty for a session with no refine model,
    /// which is the ordinary case.
    pub fn take_refine_jobs(&self) -> Vec<summo_asr::RefineJob> {
        let Ok(mut queue) = self.refine_queue.lock() else {
            return Vec::new();
        };
        queue.drain(..).collect()
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

    /// How far into the meeting each lane has got: its next sequence number and its clock.
    ///
    /// Read before a mid-meeting change tears this runner down, and handed to the one that
    /// replaces it. Sequence numbers and timestamps belong to the recording; the pipeline is only
    /// what is currently serving it, and rebuilding it used to restart both — so a swap renumbered
    /// new utterances from zero, they collided with lines already on screen, and the transcript
    /// overwrote its own beginning while the clock went back to 00:00.
    #[must_use]
    pub fn position(&self) -> HashMap<Lane, (u64, usize)> {
        self.lanes
            .iter()
            .filter_map(|(lane, runner)| {
                runner
                    .chain
                    .stage::<Recognise>()
                    .map(|r| (*lane, r.position()))
            })
            .collect()
    }

    /// Take over a meeting already in progress, lane by lane.
    ///
    /// A lane the previous runner did not have starts from nothing, which is right: it has no
    /// history in this meeting.
    pub fn resume_from(&mut self, position: &HashMap<Lane, (u64, usize)>) {
        for (lane, runner) in &mut self.lanes {
            let Some(&(seq, samples)) = position.get(lane) else {
                continue;
            };
            if let Some(stage) = runner.chain.stage_mut::<Recognise>() {
                stage.resume_at(seq, samples);
            }
        }
    }
}

/// Find an installed voice detector.
fn resolve_vad(store: &ModelStore, pinned: Option<&str>) -> Result<std::path::PathBuf> {
    let manifest = pick(store, summo_models::Task::Vad, pinned).ok_or_else(|| {
        // Coded, for the same reason `session.no_model` is: this is what a new user hits, and
        // until now they hit it as an English sentence telling them to run a command in a
        // terminal — from an app with no terminal in it. Worse, nothing showed it at all: the
        // recogniser installs, setup says ready, the timer starts and no words ever arrive,
        // because the pipeline needs a voice detector to decide where an utterance ends.
        Error::msg(
            "session.no_vad",
            "no voice detector installed. Run `summo pull silero-vad-v5`.",
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

/// The model for a role: the one that was chosen, or the only sensible one if nobody chose.
///
/// `models.vad` and `models.speaker` have been settings the models screen writes and
/// `/settings/models` accepts, and neither ever reached a recording — this took the *first*
/// installed model of the task and the choice was decoration. With one detector installed that is
/// invisible; install a second and the app records with whichever the registry happened to list
/// first, while the screen shows a tick beside the other. The same shape as `refine_model`, which
/// was disconnected in the other direction.
///
/// A pin that names something not installed falls back rather than failing. The alternative is a
/// recording refused because of a model the user removed months ago and forgot was named here, and
/// the fallback is exactly what they would have got before they ever chose.
fn pick(
    store: &ModelStore,
    task: summo_models::Task,
    pinned: Option<&str>,
) -> Option<summo_models::Manifest> {
    choose_from(&store.list(), task, pinned)
}

/// The decision itself, over a list somebody else read.
///
/// Separated from the store so it can be tested without a filesystem — the rule is four lines and
/// the part worth pinning down, and a test that has to install two voice detectors to check which
/// one wins is a test nobody writes.
fn choose_from(
    installed: &[summo_models::Manifest],
    task: summo_models::Task,
    pinned: Option<&str>,
) -> Option<summo_models::Manifest> {
    let mut of_task = installed.iter().filter(|m| m.task == task);
    let named = pinned
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .and_then(|id| of_task.clone().find(|m| m.id.as_str() == id));
    named.or_else(|| of_task.next()).cloned()
}

/// Find the speech enhancer the user asked for — and only the one they asked for.
///
/// Deliberately not [`pick`]. Every other role falls back to the first installed model of its task,
/// which is right when the role is required: a recording needs *a* voice detector, and the user who
/// installed one and never opened the models screen meant for it to be used.
///
/// Denoising is the opposite. It is optional, it changes what the decoder hears, and on clean
/// speech it makes the transcript worse. Falling back would mean that installing a denoiser to try
/// it turns it on for every meeting from then on, including the ones it hurts — a model appearing in
/// the store is not consent. Unset means off, and off is the default.
///
/// Returns `Ok(None)` when nothing is named, and an error only when something is named and cannot be
/// used, because that is a request that failed rather than a request nobody made.
fn resolve_denoise_model(
    store: &ModelStore,
    named: Option<&str>,
) -> Result<Option<std::path::PathBuf>> {
    let Some(named) = named.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };

    let manifest = store
        .list()
        .into_iter()
        .find(|m| m.task == summo_models::Task::Denoise && m.id.as_str() == named)
        .ok_or_else(|| {
            Error::ModelNotFound(format!("`{named}` is not an installed speech enhancer"))
        })?;

    let installed = store.resolve(&manifest)?;
    installed
        .param_path("model")
        .or_else(|| installed.files.values().next())
        .cloned()
        .map(Some)
        .ok_or_else(|| Error::ModelNotFound(format!("the installed `{named}` has no model file")))
}

/// Find an installed speaker-embedding model.
fn resolve_speaker_model(store: &ModelStore, pinned: Option<&str>) -> Result<std::path::PathBuf> {
    let manifest = pick(store, summo_models::Task::SpeakerEmbed, pinned).ok_or_else(|| {
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

/// What a model is for, in the words an error message needs.
///
/// Not `Debug`: `SpeakerEmbed` is a type name, and the reader of this message is somebody who put
/// the wrong id in a settings file.
fn task_words(task: summo_models::Task) -> &'static str {
    match task {
        summo_models::Task::Asr => "speech recognition",
        summo_models::Task::Vad => "voice activity detection",
        summo_models::Task::Denoise => "noise suppression",
        summo_models::Task::DiarizeSeg => "speaker segmentation",
        summo_models::Task::SpeakerEmbed => "speaker embedding",
        summo_models::Task::Embed => "text embedding",
        summo_models::Task::Translate => "translation",
    }
}

/// Load the speech model named by the session, choosing a runtime from its manifest.
pub(crate) fn load_decoder(
    id: &str,
    language: Option<&str>,
    store: &ModelStore,
    threads: usize,
) -> Result<Box<dyn Decoder>> {
    let model_id = ModelId::parse(id).map_err(Error::Config)?;
    let manifest = store.installed(&model_id)?;

    // The registry holds several kinds of model and they are not interchangeable: a voice-activity
    // detector, a speaker embedder and a translator are all installed the same way and are all
    // useless here. Nothing stops `models.live` in the settings file naming one — a user editing
    // JSON, or a copied line from a `summo pull` — and without this the failure surfaces further
    // down as an unsupported-runtime error naming a string the user never typed.
    if manifest.task != summo_models::Task::Asr {
        return Err(Error::Config(format!(
            "`{id}` is a {} model, not a speech model",
            task_words(manifest.task)
        )));
    }

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

#[cfg(test)]
mod picking {
    use super::*;

    /// The smallest manifest the parser accepts, with the two fields this decision reads.
    fn manifest(id: &str, task: summo_models::Task) -> summo_models::Manifest {
        let task = match task {
            summo_models::Task::Vad => "vad",
            summo_models::Task::Asr => "asr",
            summo_models::Task::SpeakerEmbed => "speaker-embed",
            other => panic!("this helper does not build a {other:?}"),
        };
        let json = serde_json::json!({
            "schema": 1,
            "id": id,
            "name": id,
            "task": task,
            "mode": "batch",
            "runtime": "sherpa-onnx",
            "langs": ["vi"],
            "license": "Apache-2.0",
            "attribution": "test",
            "files": [{
                "name": "model.onnx",
                "sha256": "a".repeat(64),
                "size": 1u64,
                "url": "https://example.invalid/model.onnx"
            }],
            "params": {"model": "model.onnx"}
        });
        summo_models::Manifest::parse(&json.to_string()).expect("a manifest")
    }

    /// Nobody chose, so the only installed model of the task is the answer — which is what the app
    /// did before any of this, and has to go on doing for everybody who never opens the screen.
    #[test]
    fn with_nothing_pinned_the_installed_one_is_used() {
        let only = [manifest("silero-vad-v5", summo_models::Task::Vad)];
        assert_eq!(
            choose_from(&only, summo_models::Task::Vad, None)
                .map(|m| m.id.to_string())
                .as_deref(),
            Some("silero-vad-v5")
        );
    }

    /// And a choice is honoured, which is the part that never worked: `find` took the first of the
    /// task and the pin was decoration.
    #[test]
    fn a_pinned_model_wins_over_the_first_one_listed() {
        let both = [
            manifest("silero-vad-v5", summo_models::Task::Vad),
            manifest("ten-vad", summo_models::Task::Vad),
        ];
        assert_eq!(
            choose_from(&both, summo_models::Task::Vad, Some("ten-vad"))
                .map(|m| m.id.to_string())
                .as_deref(),
            Some("ten-vad")
        );
    }

    /// A pin naming something no longer installed falls back rather than refusing the recording.
    /// Failing would end a meeting over a model the user removed months ago and forgot was named.
    #[test]
    fn a_pin_that_is_no_longer_installed_falls_back() {
        let one = [manifest("silero-vad-v5", summo_models::Task::Vad)];
        assert_eq!(
            choose_from(&one, summo_models::Task::Vad, Some("ten-vad"))
                .map(|m| m.id.to_string())
                .as_deref(),
            Some("silero-vad-v5")
        );
    }

    /// A model of the wrong task is not an answer, however precisely it was named.
    #[test]
    fn a_pin_of_the_wrong_task_is_not_used() {
        let mixed = [
            manifest("whisper-tiny", summo_models::Task::Asr),
            manifest("silero-vad-v5", summo_models::Task::Vad),
        ];
        assert_eq!(
            choose_from(&mixed, summo_models::Task::Vad, Some("whisper-tiny"))
                .map(|m| m.id.to_string())
                .as_deref(),
            Some("silero-vad-v5")
        );
    }
}
