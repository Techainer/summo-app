//! Downloading a model from inside the app.
//!
//! Setup used to mean quitting the app and running `summo setup` in a terminal, which is a
//! reasonable thing to ask of the person who wrote it and an unreasonable thing to ask of anyone
//! else. A model is hundreds of megabytes and the download is the longest thing that happens on a
//! first run, so it is a job with progress, like an import.
//!
//! Progress is polled rather than streamed, for the same reason imports are: a download started in
//! one window has to be visible in another, and reconnecting must not lose the bar.
//!
//! **A cancelled or crashed download resumes.** `Downloader` already does ranged resume against the
//! staging directory, so nothing here has to know about it — but it does mean a "failed" job is not
//! wasted work, and the message says so.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use summo_core::ModelId;

/// Where one download is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum State {
    Queued,
    /// Fetching. `done`/`total` are bytes; `total` is 0 until the first response headers arrive.
    Downloading {
        done: u64,
        total: u64,
    },
    /// Downloaded, now being verified and moved into the blob store.
    Installing,
    Done,
    Failed {
        error: String,
    },
}

impl State {
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self, State::Done | State::Failed { .. })
    }

    /// A fraction for a bar, or `None` when the size is not yet known.
    ///
    /// Not zero: a bar sitting at 0% while a 400 MB file negotiates TLS looks like a hang, and this
    /// is the first thing a new user ever sees the app do.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        match self {
            State::Downloading { done, total } if *total > 0 => {
                Some((*done as f64 / *total as f64).clamp(0.0, 1.0))
            }
            State::Done => Some(1.0),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Job {
    pub model: String,
    pub name: String,
    #[serde(flatten)]
    pub state: State,
}

/// Downloads running in this daemon, keyed by model id.
///
/// Keyed by model rather than by a job id on purpose: asking for the same model twice is a user
/// clicking a button twice, and the right answer is the job already running, not a second one
/// fighting it for the same staging file.
#[derive(Clone, Default)]
pub struct Installs {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
}

impl std::fmt::Debug for Installs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Installs")
            .field("jobs", &self.jobs.lock().len())
            .finish()
    }
}

impl Installs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a model for download.
    ///
    /// Returns the existing job if one is already running, so a double click is idempotent. A
    /// finished job is replaced, so retrying a failure works.
    pub fn claim(&self, id: &ModelId, name: &str) -> Job {
        let mut jobs = self.jobs.lock();
        let key = id.to_string();
        if let Some(existing) = jobs.get(&key)
            && !existing.state.is_finished()
        {
            return existing.clone();
        }
        let job = Job {
            model: key.clone(),
            name: name.to_string(),
            state: State::Queued,
        };
        jobs.insert(key, job.clone());
        job
    }

    pub fn set(&self, id: &str, state: State) {
        if let Some(job) = self.jobs.lock().get_mut(id) {
            job.state = state;
        }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().get(id).cloned()
    }

    /// Every download, in a stable order so a list does not reshuffle between polls.
    #[must_use]
    pub fn list(&self) -> Vec<Job> {
        let mut out: Vec<Job> = self.jobs.lock().values().cloned().collect();
        out.sort_by(|a, b| a.model.cmp(&b.model));
        out
    }

    #[must_use]
    pub fn busy(&self) -> bool {
        self.jobs.lock().values().any(|j| !j.state.is_finished())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> ModelId {
        ModelId::parse(name).unwrap()
    }

    #[test]
    fn a_claimed_model_starts_queued() {
        let installs = Installs::new();
        let job = installs.claim(&id("whisper-tiny"), "Whisper Tiny");
        assert_eq!(job.state, State::Queued);
        assert_eq!(job.name, "Whisper Tiny");
    }

    /// A user clicking Install twice must not start two downloads fighting over the same staging
    /// file.
    #[test]
    fn claiming_a_model_that_is_already_downloading_returns_the_running_job() {
        let installs = Installs::new();
        installs.claim(&id("whisper-tiny"), "a");
        installs.set(
            "whisper-tiny",
            State::Downloading {
                done: 10,
                total: 100,
            },
        );

        let again = installs.claim(&id("whisper-tiny"), "a");
        assert_eq!(
            again.state,
            State::Downloading {
                done: 10,
                total: 100
            }
        );
        assert_eq!(installs.list().len(), 1);
    }

    #[test]
    fn a_failed_download_can_be_retried() {
        let installs = Installs::new();
        installs.claim(&id("whisper-tiny"), "a");
        installs.set(
            "whisper-tiny",
            State::Failed {
                error: "mạng hỏng".into(),
            },
        );

        assert_eq!(
            installs.claim(&id("whisper-tiny"), "a").state,
            State::Queued
        );
    }

    #[test]
    fn busy_goes_false_only_once_everything_has_settled() {
        let installs = Installs::new();
        installs.claim(&id("a-model"), "a");
        installs.claim(&id("b-model"), "b");
        assert!(installs.busy());

        installs.set("a-model", State::Done);
        assert!(installs.busy());
        installs.set("b-model", State::Failed { error: "x".into() });
        assert!(!installs.busy());
    }

    /// The first thing a new user watches the app do. A bar frozen at 0% while a 400 MB file
    /// negotiates TLS reads as a hang.
    #[test]
    fn an_unknown_size_reports_no_fraction_rather_than_zero() {
        assert_eq!(State::Queued.fraction(), None);
        assert_eq!(State::Downloading { done: 0, total: 0 }.fraction(), None);
        assert_eq!(State::Installing.fraction(), None);
        assert_eq!(
            State::Downloading {
                done: 50,
                total: 200
            }
            .fraction(),
            Some(0.25)
        );
    }

    #[test]
    fn more_bytes_than_expected_still_reads_as_complete() {
        assert_eq!(
            State::Downloading {
                done: 201,
                total: 200
            }
            .fraction(),
            Some(1.0)
        );
    }

    #[test]
    fn the_list_does_not_reshuffle_between_polls() {
        let installs = Installs::new();
        installs.claim(&id("z-model"), "z");
        installs.claim(&id("a-model"), "a");
        let names: Vec<_> = installs.list().into_iter().map(|j| j.model).collect();
        assert_eq!(names, ["a-model", "z-model"]);
    }
}
