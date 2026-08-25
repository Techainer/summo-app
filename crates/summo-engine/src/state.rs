//! What the daemon knows between requests.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};
use summo_models::{ModelStore, hw::HwProfile};

use crate::protocol::SessionSpec;

/// Whether a recording is in progress, and with what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Recording {
        /// Seconds of audio accepted so far.
        elapsed_s: f64,
        live_model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        refine_model: Option<String>,
        /// The speech enhancer cleaning each utterance, when one was chosen.
        ///
        /// Reported for the reason every other model here is: a setting that changes what the
        /// decoder hears and cannot be seen from outside is how `models.refine`, `models.vad` and
        /// `models.speaker` each spent a release doing nothing while their screens showed a tick.
        #[serde(skip_serializing_if = "Option::is_none")]
        denoise_model: Option<String>,
        /// The language this session resolved to, or `None` for the model's own detection.
        ///
        /// Here because the interface has to be able to *say* what it is hearing, and its own copy
        /// of the preference is not that: a session started with no language named resolves to the
        /// daemon's setting, and a banner reading it from the browser would announce "detecting"
        /// while the daemon confidently decoded Vietnamese.
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        /// What finished lines are being translated into. Empty means no translation.
        ///
        /// Reported for the same reason as `language`, and it was the half that was missing: the
        /// in-meeting banner could say what was being heard and had no way to say whether anything
        /// was being translated, so the one visible sign that translation was on at all was a
        /// second line of text appearing under the first — or not appearing, with nothing on screen
        /// to say why.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        translate_into: Vec<String>,
        /// Utterances committed so far.
        segments: u64,
        /// The document this session is writing into.
        ///
        /// The id has always existed — it is minted when the recording starts and the file is
        /// named after it — and it has never left the daemon, so the interface could not open the
        /// meeting it was in the middle of. That is why recording had a screen of its own with a
        /// transcript and nowhere to type: there was no way to point at the page.
        ///
        /// `Option`, because a build without the `models` feature accepts a session and has no
        /// pipeline to write one — there is genuinely no document, and saying so is better than an
        /// id that leads nowhere.
        #[serde(skip_serializing_if = "Option::is_none")]
        meeting: Option<summo_core::MeetingId>,
    },
}

impl SessionStatus {
    /// The document a running session is writing into, if there is one.
    #[must_use]
    pub fn meeting(&self) -> Option<&summo_core::MeetingId> {
        match self {
            Self::Recording { meeting, .. } => meeting.as_ref(),
            Self::Idle => None,
        }
    }
}

impl SessionStatus {
    #[must_use]
    pub fn is_recording(&self) -> bool {
        matches!(self, Self::Recording { .. })
    }
}

/// Shared daemon state.
///
/// Cloning is cheap and shares the same underlying state; every handler gets a clone.
#[derive(Clone)]
pub struct EngineState {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    hw: HwProfile,
    status: RwLock<SessionStatus>,
    imports: crate::imports::Imports,
    installs: crate::install::Installs,
    /// One speech model kept loaded, so pressing record does not wait 3.4 seconds for one.
    #[cfg(feature = "models")]
    warm: crate::warm::Warm,
}

impl EngineState {
    pub fn new(paths: Paths) -> Result<Self> {
        paths.ensure()?;

        // The models the installer shipped with, on a vault that has never seen any.
        //
        // Before this, the first thing a new install did was ask for the network: measure the
        // machine, list the models, wait for seventy megabytes. On a connection where the registry
        // is blocked — an ordinary Vietnamese ISP — the screen said "Could not reach the model
        // list" and there was nothing else to press. An app somebody downloaded should run.
        //
        // Never fatal. A daemon that refuses to start because a bundled file is missing is worse
        // than one that starts and offers to download; the setup screen already knows how to say
        // that no model is installed.
        match summo_models::seed::seed(&paths) {
            Ok(seeded) if !seeded.installed.is_empty() => {
                tracing::info!(models = ?seeded.installed, "installed the models that shipped with the app");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "could not install the bundled models"),
        }

        Ok(Self {
            inner: Arc::new(Inner {
                paths,
                hw: HwProfile::detect(),
                status: RwLock::new(SessionStatus::Idle),
                imports: crate::imports::Imports::new(),
                installs: crate::install::Installs::new(),
                #[cfg(feature = "models")]
                warm: crate::warm::Warm::default(),
            }),
        })
    }

    /// The speech model kept loaded between recordings.
    ///
    /// On the state rather than inside the session, because its whole purpose is to exist when no
    /// session does — a decoder built while nothing is recording is what makes the next recording
    /// start immediately.
    #[cfg(feature = "models")]
    #[must_use]
    pub fn warm(&self) -> &crate::warm::Warm {
        &self.inner.warm
    }

    /// Imports running in this daemon. Shared, so a job started from one window is visible in
    /// every other one and in the CLI.
    #[must_use]
    pub fn imports(&self) -> &crate::imports::Imports {
        &self.inner.imports
    }

    /// Model downloads running in this daemon. Shared for the same reason imports are: a download
    /// started in one window has to be visible in every other.
    #[must_use]
    pub fn installs(&self) -> &crate::install::Installs {
        &self.inner.installs
    }

    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.inner.paths
    }

    #[must_use]
    pub fn hardware(&self) -> &HwProfile {
        &self.inner.hw
    }

    #[must_use]
    pub fn store(&self) -> ModelStore {
        ModelStore::new(self.inner.paths.clone())
    }

    #[must_use]
    pub fn status(&self) -> SessionStatus {
        self.inner.status.read().clone()
    }

    /// Move to recording. Fails if a session is already running.
    ///
    /// Two concurrent sessions would fight over the microphone and interleave their events on one
    /// socket, so this is a hard error rather than a silent replacement.
    pub fn begin(&self, spec: &SessionSpec, meeting: Option<summo_core::MeetingId>) -> Result<()> {
        let mut status = self.inner.status.write();
        if status.is_recording() {
            return Err(Error::Other("a recording is already in progress".into()));
        }
        *status = SessionStatus::Recording {
            elapsed_s: 0.0,
            live_model: spec.live_model.clone(),
            refine_model: spec.refine_model.clone(),
            denoise_model: spec.denoise_model.clone(),
            language: spec.language.clone(),
            translate_into: spec.translate_into.clone(),
            segments: 0,
            meeting,
        };
        Ok(())
    }

    /// Say what a running session is listening with now, after a mid-meeting change.
    ///
    /// Silently ignored when nothing is recording: a swap that arrives as the meeting ends is a
    /// race, not a mistake, and there is no state left to correct.
    pub fn retuned(&self, spec: &SessionSpec) {
        let mut status = self.inner.status.write();
        if let SessionStatus::Recording {
            live_model,
            refine_model,
            denoise_model,
            language,
            translate_into,
            ..
        } = &mut *status
        {
            live_model.clone_from(&spec.live_model);
            refine_model.clone_from(&spec.refine_model);
            denoise_model.clone_from(&spec.denoise_model);
            language.clone_from(&spec.language);
            translate_into.clone_from(&spec.translate_into);
        }
    }

    /// Record progress, for the status endpoint and the performance HUD.
    pub fn advance(&self, added_s: f64, added_segments: u64) {
        let mut status = self.inner.status.write();
        if let SessionStatus::Recording {
            elapsed_s,
            segments,
            ..
        } = &mut *status
        {
            *elapsed_s += added_s;
            *segments += added_segments;
        }
    }

    /// Return to idle. Idempotent: stopping a session that already ended is not an error, because a
    /// client that reconnects after a drop cannot know whether its stop arrived.
    pub fn end(&self) -> SessionStatus {
        let mut status = self.inner.status.write();
        let previous = status.clone();
        *status = SessionStatus::Idle;
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, EngineState) {
        let tmp = tempfile::tempdir().unwrap();
        let state = EngineState::new(Paths::at(tmp.path())).unwrap();
        (tmp, state)
    }

    #[test]
    fn a_fresh_engine_is_idle() {
        let (_tmp, state) = state();
        assert_eq!(state.status(), SessionStatus::Idle);
        assert!(!state.status().is_recording());
    }

    #[test]
    fn beginning_a_session_records_which_models_are_in_use() {
        let (_tmp, state) = state();
        let mut spec = SessionSpec::new("gipformer-65m");
        spec.refine_model = Some("whisper-large-v3".into());

        state.begin(&spec, None).unwrap();

        let SessionStatus::Recording {
            live_model,
            refine_model,
            ..
        } = state.status()
        else {
            panic!("expected a recording state")
        };
        assert_eq!(live_model, "gipformer-65m");
        assert_eq!(refine_model.as_deref(), Some("whisper-large-v3"));
    }

    #[test]
    fn a_second_session_cannot_start_while_one_is_running() {
        // Two sessions would fight over the microphone and interleave events on one socket.
        let (_tmp, state) = state();
        let spec = SessionSpec::new("m");
        state.begin(&spec, None).unwrap();
        assert!(state.begin(&spec, None).is_err());
    }

    #[test]
    fn progress_accumulates_while_recording() {
        let (_tmp, state) = state();
        state.begin(&SessionSpec::new("m"), None).unwrap();
        state.advance(1.5, 1);
        state.advance(2.0, 2);

        let SessionStatus::Recording {
            elapsed_s,
            segments,
            ..
        } = state.status()
        else {
            panic!("expected a recording state")
        };
        assert!((elapsed_s - 3.5).abs() < 1e-9);
        assert_eq!(segments, 3);
    }

    #[test]
    fn progress_reported_while_idle_is_ignored() {
        let (_tmp, state) = state();
        state.advance(10.0, 5);
        assert_eq!(state.status(), SessionStatus::Idle);
    }

    #[test]
    fn stopping_twice_is_not_an_error() {
        // A client that reconnects after a dropped socket cannot know whether its stop arrived.
        let (_tmp, state) = state();
        state.begin(&SessionSpec::new("m"), None).unwrap();
        assert!(state.end().is_recording());
        assert_eq!(state.end(), SessionStatus::Idle);
    }

    #[test]
    fn a_session_can_start_again_after_stopping() {
        let (_tmp, state) = state();
        let spec = SessionSpec::new("m");
        state.begin(&spec, None).unwrap();
        state.end();
        assert!(state.begin(&spec, None).is_ok());
    }

    #[test]
    fn clones_share_one_state() {
        let (_tmp, state) = state();
        let handle = state.clone();
        state.begin(&SessionSpec::new("m"), None).unwrap();
        assert!(
            handle.status().is_recording(),
            "handlers must see the same session"
        );
    }
}
