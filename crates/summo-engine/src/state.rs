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
        /// Utterances committed so far.
        segments: u64,
    },
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
}

impl EngineState {
    pub fn new(paths: Paths) -> Result<Self> {
        paths.ensure()?;
        Ok(Self {
            inner: Arc::new(Inner {
                paths,
                hw: HwProfile::detect(),
                status: RwLock::new(SessionStatus::Idle),
                imports: crate::imports::Imports::new(),
            }),
        })
    }

    /// Imports running in this daemon. Shared, so a job started from one window is visible in
    /// every other one and in the CLI.
    #[must_use]
    pub fn imports(&self) -> &crate::imports::Imports {
        &self.inner.imports
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
    pub fn begin(&self, spec: &SessionSpec) -> Result<()> {
        let mut status = self.inner.status.write();
        if status.is_recording() {
            return Err(Error::Other("a recording is already in progress".into()));
        }
        *status = SessionStatus::Recording {
            elapsed_s: 0.0,
            live_model: spec.live_model.clone(),
            refine_model: spec.refine_model.clone(),
            segments: 0,
        };
        Ok(())
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

        state.begin(&spec).unwrap();

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
        state.begin(&spec).unwrap();
        assert!(state.begin(&spec).is_err());
    }

    #[test]
    fn progress_accumulates_while_recording() {
        let (_tmp, state) = state();
        state.begin(&SessionSpec::new("m")).unwrap();
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
        state.begin(&SessionSpec::new("m")).unwrap();
        assert!(state.end().is_recording());
        assert_eq!(state.end(), SessionStatus::Idle);
    }

    #[test]
    fn a_session_can_start_again_after_stopping() {
        let (_tmp, state) = state();
        let spec = SessionSpec::new("m");
        state.begin(&spec).unwrap();
        state.end();
        assert!(state.begin(&spec).is_ok());
    }

    #[test]
    fn clones_share_one_state() {
        let (_tmp, state) = state();
        let handle = state.clone();
        state.begin(&SessionSpec::new("m")).unwrap();
        assert!(
            handle.status().is_recording(),
            "handlers must see the same session"
        );
    }
}
