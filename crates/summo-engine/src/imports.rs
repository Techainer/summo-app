//! Turning a file the user already has into a meeting, in the background.
//!
//! Importing is the one operation in Summo that is both slow and unattended: a two-hour recording
//! takes minutes to decode, and the user is not going to sit and watch. So it is a *job* rather
//! than a request — the caller gets an id immediately, and asks how it is going.
//!
//! Progress is polled, not pushed. The WebSocket in this daemon carries one recording session for
//! one client; putting import progress on it would mean an import started from the CLI is invisible
//! to the app, and an app that reconnects loses the progress of a job still running. A registry the
//! job writes into and anyone reads from does not have either problem.
//!
//! **Failure is recorded, not swallowed.** A job that dies leaves a [`JobState::Failed`] with the
//! message, because the alternative — a job that quietly disappears — is indistinguishable from one
//! that is still working, and the user waits forever.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, paths::Paths};

/// Where a job is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum JobState {
    /// Accepted, not yet started — another import is ahead of it.
    Queued,
    /// Pulling the audio out of the container. No useful fraction: ffmpeg's own progress is not
    /// worth parsing for a step that is a few percent of the total.
    Extracting,
    /// Decoding.
    Running {
        done_s: f64,
        total_s: f64,
        segments: usize,
    },
    /// Finished, with the meeting it produced.
    Done {
        meeting: String,
        path: String,
        segments: usize,
        duration_s: f64,
    },
    Failed {
        error: String,
    },
}

impl JobState {
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self, JobState::Done { .. } | JobState::Failed { .. })
    }

    /// Fraction complete, for a progress bar. `None` while the length is not yet known.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        match self {
            JobState::Running {
                done_s, total_s, ..
            } if *total_s > 0.0 => Some((done_s / total_s).clamp(0.0, 1.0)),
            JobState::Done { .. } => Some(1.0),
            _ => None,
        }
    }
}

/// One import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// What the user will recognise it by — the file's name, cleaned up.
    pub title: String,
    /// The file it came from, so a failure can be reported against something the user recognises.
    pub source: String,
    #[serde(flatten)]
    pub state: JobState,
}

/// Every import this daemon has run since it started.
///
/// Not persisted. A daemon restart loses the *history*, which is fine, because the meeting a
/// finished job produced is on disk and a job interrupted by a restart is gone anyway — recording
/// its final state would only promise a resume that does not exist.
#[derive(Clone, Default)]
pub struct Imports {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    /// Insertion order, so the list reads newest-first without sorting on a field that may tie.
    order: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Debug for Imports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Imports")
            .field("jobs", &self.order.lock().len())
            .finish()
    }
}

impl Imports {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a job and return its id.
    pub fn add(&self, title: impl Into<String>, source: &Path) -> String {
        let id = MeetingId::new().to_string();
        let job = Job {
            id: id.clone(),
            title: title.into(),
            source: source.display().to_string(),
            state: JobState::Queued,
        };
        self.jobs.lock().insert(id.clone(), job);
        self.order.lock().push(id.clone());
        id
    }

    pub fn set(&self, id: &str, state: JobState) {
        if let Some(job) = self.jobs.lock().get_mut(id) {
            job.state = state;
        }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().get(id).cloned()
    }

    /// Every job, newest first.
    #[must_use]
    pub fn list(&self) -> Vec<Job> {
        let jobs = self.jobs.lock();
        self.order
            .lock()
            .iter()
            .rev()
            .filter_map(|id| jobs.get(id).cloned())
            .collect()
    }

    /// Whether anything is still working. Used to keep a UI polling only while it needs to.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.jobs.lock().values().any(|j| !j.state.is_finished())
    }

    /// Drop finished jobs, returning how many went.
    pub fn clear_finished(&self) -> usize {
        let mut jobs = self.jobs.lock();
        let mut order = self.order.lock();
        let before = jobs.len();
        jobs.retain(|_, j| !j.state.is_finished());
        order.retain(|id| jobs.contains_key(id));
        before - jobs.len()
    }
}

/// Check a path is something worth importing before a job is created.
///
/// Done up front so the caller gets a real error instead of a job id that fails a second later, and
/// so a typo'd path never reaches ffmpeg.
pub fn check(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(Error::msg(
            "import.not_found",
            format!("không thấy {}", path.display()),
        ));
    }
    if path.is_dir() {
        return Err(Error::msg(
            "import.is_directory",
            format!("{} là thư mục; nhập từng file một", path.display()),
        ));
    }
    Ok(())
}

/// Where a job's extracted audio lives.
///
/// Beside the recorded lanes of the meeting it becomes, so forgetting a meeting's audio deletes the
/// import too rather than leaving an orphan the storage screen cannot see.
#[must_use]
pub fn audio_path(paths: &Paths, meeting: &MeetingId) -> PathBuf {
    paths.audio_for(meeting).join("import.wav")
}

/// What to import, and what to do with it afterwards.
///
/// A struct because the alternative was an eighth positional `bool` on two functions, and because
/// the file to decode and where it came from stopped being the same thing once a link could be
/// imported: the bytes are in a download, the provenance is a URL, and a meeting should record the
/// one a person would recognise.
#[derive(Debug, Clone)]
pub struct Source {
    /// The file to decode.
    pub file: PathBuf,
    /// What to record in the meeting as where it came from: the path, or the URL it was fetched
    /// from.
    pub origin: String,
    /// Copy it into the vault, so watching it back survives the original being moved.
    pub keep: bool,
    /// Delete `file` when the job ends, however it ends.
    ///
    /// True for a download, which is staging and not the user's file. Without it a link import
    /// leaves two copies of the same recording on disk — one in the vault where it belongs and one
    /// in `~/.summo/downloads` that nothing will ever look at again.
    pub cleanup: bool,
}

impl Source {
    /// A file already on this machine, left where it is.
    #[must_use]
    pub fn file(path: &Path) -> Self {
        Self {
            file: path.to_path_buf(),
            origin: path.display().to_string(),
            keep: false,
            cleanup: false,
        }
    }

    #[must_use]
    pub fn keeping(mut self, keep: bool) -> Self {
        self.keep = keep;
        self
    }
}

/// Copy the imported file into the vault, so playback survives the original being moved.
///
/// Beside the extracted audio, under a fixed stem, so forgetting a meeting's audio takes the copy
/// with it rather than leaving a gigabyte the storage screen cannot account for. The extension is
/// kept because it is how a browser decides what it can play.
///
/// The original's own extension and nothing from its name: a file called `../../x.mp4` is a path,
/// and this is a place where a user-supplied string would otherwise reach the filesystem.
///
/// Behind `models` with the rest of importing: a build with no recogniser has nothing to import
/// from and so nothing to keep a copy of.
#[cfg(feature = "models")]
fn keep_a_copy(paths: &Paths, meeting: &MeetingId, source: &Path) -> Result<PathBuf> {
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin")
        .to_ascii_lowercase();

    let dir = paths.audio_for(meeting);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let target = dir.join(format!(
        "{}.{extension}",
        crate::audio_stream::KEPT_SOURCE_STEM
    ));
    std::fs::copy(source, &target).map_err(|e| Error::io(&target, e))?;
    Ok(target)
}

/// The day a file belongs to, from when it was written rather than when it was imported.
///
/// A Zoom recording from last Tuesday is *last Tuesday's* meeting. Filing it under today would put
/// it above meetings that actually happened today and break every report that groups by day. Falls
/// back to today only when the filesystem cannot say.
#[must_use]
pub fn day_of(path: &Path) -> String {
    use time::OffsetDateTime;

    let from_file = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(OffsetDateTime::from)
        .and_then(|utc| {
            // The stored time is UTC; the day the user calls it is the day in their own zone.
            time::UtcOffset::current_local_offset()
                .map(|offset| utc.to_offset(offset))
                .ok()
        });

    let now = from_file.unwrap_or_else(|| {
        OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
    });
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

/// Extract, decode and file one recording. Blocking — run it on its own thread.
///
/// Every state change goes through `imports` as it happens, so a caller polling the registry sees
/// extraction, then decoding progress, then the meeting. A failure at any point lands as
/// [`JobState::Failed`] rather than propagating, because the caller is a detached thread and there
/// is nowhere for an error to go.
#[cfg(feature = "models")]
pub fn run(
    imports: &Imports,
    job_id: &str,
    paths: &Paths,
    store: &summo_models::ModelStore,
    hw: &summo_models::hw::HwProfile,
    spec: &crate::protocol::SessionSpec,
    source: &Source,
) {
    let outcome = execute(imports, job_id, paths, store, hw, spec, source);

    // However it went. A download that failed halfway is still a download, and leaving it behind
    // means the one thing a user cannot see — `~/.summo/downloads` — is the one that grows.
    if source.cleanup
        && let Err(e) = std::fs::remove_file(&source.file)
    {
        tracing::warn!(error = %e, path = %source.file.display(), "could not remove the download");
    }

    match outcome {
        Ok(state) => imports.set(job_id, state),
        Err(e) => imports.set(
            job_id,
            JobState::Failed {
                error: e.to_string(),
            },
        ),
    }
}

#[cfg(feature = "models")]
fn execute(
    imports: &Imports,
    job_id: &str,
    paths: &Paths,
    store: &summo_models::ModelStore,
    hw: &summo_models::hw::HwProfile,
    spec: &crate::protocol::SessionSpec,
    source: &Source,
) -> Result<JobState> {
    let file = source.file.as_path();
    check(file)?;
    imports.set(job_id, JobState::Extracting);

    // No `probe()` here any more. Requiring ffmpeg to *exist* before looking at a file meant an
    // import failed on a machine that could have decoded it perfectly well in this process.
    let info = summo_media::info_of(file)?;
    if !info.has_audio {
        return Err(Error::msg(
            "import.no_audio",
            format!("{} không có âm thanh", file.display()),
        ));
    }

    // The meeting id is minted here, before anything is written, because the extracted audio is
    // stored under it — the file and the note have to agree on which meeting they belong to.
    let id = MeetingId::new();
    let wav_path = audio_path(paths, &id);
    summo_media::to_wav16(file, &wav_path)?;

    let wav = crate::offline::read_wav(&wav_path)?;
    let mut runner = crate::runner::SessionRunner::new(spec, store, hw)?;

    let title = summo_media::title_from(file);
    let day = day_of(file);
    let models = vec![("live".to_string(), spec.live_model.clone())];

    // The document is opened **before** decoding, the way a recording opens one the moment it
    // starts, and for the same reasons. It used to be created after the last sample: the whole
    // transcript of a two-hour file lived in memory until then, so nothing could be read while the
    // import ran and a daemon that died forty minutes in had produced nothing at all.
    //
    // Now the meeting exists from the first second — with a title, a date, and where it came from —
    // and fills in as the file decodes. An import can be read while it happens.
    let meeting = id.to_string();
    let mut recorder = crate::recorder::Recorder::start(paths, id.clone(), &title, &day, models)?;
    recorder.set_source(&source.origin);

    // The copy is made before decoding, not after, so a two-hour import does not end by discovering
    // that the drive it came from was unplugged an hour ago. It is opt-in: an mp4 of a long meeting
    // is measured in gigabytes, and quietly doubling it inside somebody's vault is not a default.
    if source.keep
        && let Err(e) = keep_a_copy(paths, &id, file)
    {
        tracing::warn!(error = %e, "could not keep a copy of the imported file");
    }

    crate::offline::transcribe(&wav, &mut runner, |p, events| {
        for event in events {
            recorder.apply(event);
        }
        // Autosave decides how often this actually reaches the disk; offering it the chance per
        // block is what makes the file track the decode rather than appear at the end of it. A
        // failed write is logged and the decode continues — losing the rest of a two-hour import
        // because one save failed is the wrong trade.
        if let Err(e) = recorder.maybe_save() {
            tracing::warn!(error = %e, "could not write the import's progress");
        }
        imports.set(
            job_id,
            JobState::Running {
                done_s: p.done_s,
                total_s: p.total_s,
                segments: p.segments,
            },
        );
        // Nothing cancels an import yet; when it does, this is where the flag is read.
        true
    })?;

    let segments = recorder.segment_count();
    let duration_s = wav.duration_s();
    let path = recorder.finish(duration_s)?;

    Ok(JobState::Done {
        meeting,
        path: path.display().to_string(),
        segments,
        duration_s,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_starts_queued_and_is_findable_by_id() {
        let imports = Imports::new();
        let id = imports.add("Họp tuần", Path::new("/tmp/a.mp4"));
        let job = imports.get(&id).expect("job");
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.title, "Họp tuần");
    }

    #[test]
    fn the_newest_job_is_listed_first() {
        let imports = Imports::new();
        imports.add("một", Path::new("/tmp/1.mp4"));
        imports.add("hai", Path::new("/tmp/2.mp4"));
        let titles: Vec<_> = imports.list().into_iter().map(|j| j.title).collect();
        assert_eq!(titles, ["hai", "một"]);
    }

    /// A UI that polls forever costs battery; a UI that stops polling too early leaves a spinner
    /// running. Both hang on this one predicate.
    #[test]
    fn busy_goes_false_only_once_every_job_has_settled() {
        let imports = Imports::new();
        let a = imports.add("a", Path::new("/tmp/a.mp4"));
        let b = imports.add("b", Path::new("/tmp/b.mp4"));
        assert!(imports.busy());

        imports.set(
            &a,
            JobState::Done {
                meeting: "m".into(),
                path: "p".into(),
                segments: 3,
                duration_s: 10.0,
            },
        );
        assert!(imports.busy(), "one job left");

        imports.set(
            &b,
            JobState::Failed {
                error: "hỏng".into(),
            },
        );
        assert!(!imports.busy(), "a failure settles a job too");
    }

    #[test]
    fn clearing_keeps_the_jobs_still_running() {
        let imports = Imports::new();
        let done = imports.add("xong", Path::new("/tmp/a.mp4"));
        imports.add("đang chạy", Path::new("/tmp/b.mp4"));
        imports.set(&done, JobState::Failed { error: "x".into() });

        assert_eq!(imports.clear_finished(), 1);
        let left: Vec<_> = imports.list().into_iter().map(|j| j.title).collect();
        assert_eq!(left, ["đang chạy"]);
        assert!(imports.get(&done).is_none());
    }

    #[test]
    fn progress_reads_as_a_fraction_only_once_the_length_is_known() {
        assert_eq!(JobState::Queued.fraction(), None);
        assert_eq!(JobState::Extracting.fraction(), None);
        assert_eq!(
            JobState::Running {
                done_s: 0.0,
                total_s: 0.0,
                segments: 0
            }
            .fraction(),
            None,
            "a file whose length ffmpeg did not report must not read as 0%, which looks stuck"
        );
        assert_eq!(
            JobState::Running {
                done_s: 30.0,
                total_s: 60.0,
                segments: 1
            }
            .fraction(),
            Some(0.5)
        );
    }

    #[test]
    fn a_missing_file_is_refused_before_a_job_exists() {
        let err = check(Path::new("/nonexistent/x.mp4"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("không thấy"), "{err}");
    }

    #[test]
    fn a_folder_is_refused_with_advice_rather_than_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = check(dir.path()).unwrap_err().to_string();
        assert!(err.contains("thư mục"), "{err}");
    }

    /// The import's audio has to live where "forget this meeting's audio" already looks, or it
    /// becomes a file the storage screen reports as missing and never reclaims.
    #[test]
    fn extracted_audio_lands_in_the_meeting_s_own_audio_folder() {
        let paths = Paths::at("/tmp/summo-test");
        let id = MeetingId::new();
        assert!(audio_path(&paths, &id).starts_with(paths.audio_for(&id)));
    }
}
