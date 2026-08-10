//! Attributing utterances to people, and changing that answer later.
//!
//! The pipeline is four independent steps, in their own thread, one utterance at a time:
//!
//! ```text
//!   final segment ──► embed ──► identify against the voice book ──► label ──► store the vector
//! ```
//!
//! Two properties fall out of that shape and both matter.
//!
//! **Nothing blocks the transcript.** Attribution runs after a segment is already final and already
//! on screen, so a slow embedding costs a label arriving a second late — not a word appearing a
//! second late. Speaker names filling in behind the text is normal; text stuttering is not.
//!
//! **The vector is kept, not just the label.** That is what makes correction cheap. When a user
//! finally puts a name to "Người 2", nothing has to be re-run: every stored vector is compared
//! against the updated book with a loop and a dot product, and every past meeting is relabelled in
//! milliseconds. Storing only labels would mean either re-reading all the audio or accepting that
//! history stays wrong.
//!
//! A vector is about 768 bytes. Ten thousand utterances is under 8 MB — nothing next to the audio.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, SpeakerId};

use crate::voices::{Match, VoiceBook, unknown_speaker};

/// Schema version, so a future embedding model can be detected rather than compared against.
const SCHEMA: u32 = 1;

/// One attributed utterance, with the vector that produced the attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceSample {
    pub seq: u64,
    pub t0: f64,
    pub duration: f64,
    pub embedding: Vec<f32>,
    /// Person id from the voice book, when one matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    /// The label shown in the transcript, named or not.
    pub label: SpeakerId,
    /// True when a human set this, and no amount of matching may overturn it.
    #[serde(default)]
    pub confirmed: bool,
}

/// Every attributed utterance of one meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceLog {
    pub meeting: MeetingId,
    #[serde(default = "schema")]
    pub schema: u32,
    /// Which model produced these vectors. Comparing across models is meaningless, so a change
    /// invalidates the log rather than silently producing nonsense.
    #[serde(default)]
    pub model: String,
    pub samples: Vec<VoiceSample>,
}

fn schema() -> u32 {
    SCHEMA
}

impl VoiceLog {
    #[must_use]
    pub fn new(meeting: MeetingId, model: impl Into<String>) -> Self {
        Self {
            meeting,
            schema: SCHEMA,
            model: model.into(),
            samples: Vec::new(),
        }
    }

    /// Where a meeting's vectors live.
    ///
    /// Under `voices/`, not next to the audio: retention deletes recordings after a month, and
    /// these have to outlive that — they are what lets a name applied next year fix last year's
    /// transcripts.
    #[must_use]
    pub fn path_for(voices_dir: &Path, meeting: &MeetingId) -> PathBuf {
        voices_dir.join("meetings").join(format!("{}.json", meeting.as_str()))
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let log: Self = serde_json::from_str(&text)
                    .map_err(|e| Error::Other(format!("cannot parse {}: {e}", path.display())))?;
                if log.schema > SCHEMA {
                    return Err(Error::Other(format!(
                        "{} was written by a newer build",
                        path.display()
                    )));
                }
                Ok(Some(log))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(self)?)
            .map_err(|e| Error::io(&temporary, e))?;
        std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))
    }

    /// Distinct labels in this meeting, in first-heard order.
    #[must_use]
    pub fn labels(&self) -> Vec<SpeakerId> {
        let mut out: Vec<SpeakerId> = Vec::new();
        for sample in &self.samples {
            if !out.contains(&sample.label) {
                out.push(sample.label.clone());
            }
        }
        out
    }

    /// Every vector currently carrying `label`, which is what a user names when they pick a person.
    #[must_use]
    pub fn embeddings_for(&self, label: &SpeakerId) -> Vec<Vec<f32>> {
        self.samples
            .iter()
            .filter(|s| &s.label == label)
            .map(|s| s.embedding.clone())
            .collect()
    }
}

/// One label change, for the transcript to apply.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Relabel {
    pub seq: u64,
    pub from: SpeakerId,
    pub to: SpeakerId,
}

/// Attributes utterances as they finish, one meeting at a time.
///
/// Holds a copy of the voice book rather than a lock on it: the book changes when a user names
/// somebody, which is rare, and reading it per utterance would put the interface's edits on the
/// path of the recording.
pub struct Attributor {
    book: VoiceBook,
    log: VoiceLog,
    /// Voices in this meeting nobody has named, in the order they were first heard.
    unknown: Vec<Vec<f32>>,
    min_duration_s: f64,
    same_voice: f32,
}

impl Attributor {
    #[must_use]
    pub fn new(book: VoiceBook, log: VoiceLog) -> Self {
        Self {
            book,
            log,
            unknown: Vec::new(),
            // Shorter than this carries too little voice to embed reliably; a label guessed from
            // "vâng" is a coin flip wearing a name.
            min_duration_s: 0.8,
            same_voice: 0.62,
        }
    }

    /// Attribute one finished utterance. Returns the label to show, if any.
    ///
    /// A short utterance gets no label rather than a guess. In a meeting most one-word replies
    /// belong to whoever was already speaking, and the transcript reads fine without a name on
    /// them.
    pub fn attribute(&mut self, seq: u64, t0: f64, duration: f64, embedding: &[f32]) -> Option<SpeakerId> {
        if duration < self.min_duration_s || embedding.is_empty() {
            return None;
        }

        let (person, label) = match self.book.identify(embedding) {
            Match::Known { person, .. } => {
                let name = self
                    .book
                    .get(&person)
                    .map_or_else(|| person.clone(), |p| p.name.clone());
                (Some(person), SpeakerId::from(name))
            }
            // Unsure and Unknown are treated alike on purpose: both mean "do not put a name on
            // this", and the difference only matters to a diagnostic.
            Match::Unsure { .. } | Match::Unknown { .. } => {
                let index = self.unknown_index(embedding);
                (None, unknown_speaker(index + 1))
            }
        };

        self.log.samples.push(VoiceSample {
            seq,
            t0,
            duration,
            embedding: embedding.to_vec(),
            person,
            label: label.clone(),
            confirmed: false,
        });
        Some(label)
    }

    /// Which unnamed voice of this meeting this is, adding one if it is new.
    fn unknown_index(&mut self, embedding: &[f32]) -> usize {
        let best = self
            .unknown
            .iter()
            .enumerate()
            .map(|(i, c)| (i, cosine(c, embedding)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        if let Some((i, similarity)) = best
            && similarity >= self.same_voice
        {
            // Drift the centroid slowly: one utterance recorded while somebody leaned away from
            // the microphone should not drag the whole voice with it.
            for (slot, sample) in self.unknown[i].iter_mut().zip(embedding) {
                *slot = *slot * 0.85 + sample * 0.15;
            }
            return i;
        }
        self.unknown.push(embedding.to_vec());
        self.unknown.len() - 1
    }

    #[must_use]
    pub fn log(&self) -> &VoiceLog {
        &self.log
    }

    #[must_use]
    pub fn into_log(self) -> VoiceLog {
        self.log
    }
}

/// Recompute every label in a log against the book as it stands now.
///
/// Pure arithmetic over stored vectors — no audio is read and no model runs — which is what makes
/// naming a voice cheap enough to do retroactively across a whole history.
///
/// A confirmed sample is never changed. A user who told Summo who was speaking has settled it, and
/// a later match that disagrees is the thing that is wrong.
#[must_use]
pub fn relabel(log: &VoiceLog, book: &VoiceBook) -> Vec<Relabel> {
    let mut changes = Vec::new();
    for sample in &log.samples {
        if sample.confirmed {
            continue;
        }
        if let Match::Known { person, .. } = book.identify(&sample.embedding) {
            let name = book
                .get(&person)
                .map_or_else(|| person.clone(), |p| p.name.clone());
            let to = SpeakerId::from(name);
            if to != sample.label {
                changes.push(Relabel {
                    seq: sample.seq,
                    from: sample.label.clone(),
                    to,
                });
            }
        }
    }
    changes
}

/// Apply relabelling to the log itself, so the next sweep starts from the new answer.
pub fn apply(log: &mut VoiceLog, book: &VoiceBook, changes: &[Relabel]) {
    for change in changes {
        if let Some(sample) = log.samples.iter_mut().find(|s| s.seq == change.seq) {
            sample.label = change.to.clone();
            sample.person = book
                .people()
                .find(|p| p.name == change.to.as_str())
                .map(|p| p.id.clone());
        }
    }
}

/// Record that a human said these utterances are `person`.
///
/// Confirmed samples are the ground truth the book learns from, and they are immune to later
/// matching — the whole point of asking a person is that their answer wins.
pub fn confirm(log: &mut VoiceLog, label: &SpeakerId, person_id: &str, name: &str) {
    let to = SpeakerId::from(name.to_string());
    for sample in log.samples.iter_mut().filter(|s| &s.label == label) {
        sample.label = to.clone();
        sample.person = Some(person_id.to_string());
        sample.confirmed = true;
    }
}

/// Relabel every meeting on disk against the current book.
///
/// This is the sweep that runs after somebody names a voice: every past `Người 2` that is really
/// them becomes their name. Reported per meeting so a caller can rewrite only the files that
/// changed.
pub fn resweep(voices_dir: &Path, book: &VoiceBook) -> Result<Vec<(MeetingId, Vec<Relabel>)>> {
    let dir = voices_dir.join("meetings");
    let mut out = Vec::new();

    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(mut log) = VoiceLog::load(&path).unwrap_or_else(|e| {
            // One unreadable log must not stop the rest of the history being corrected.
            tracing::warn!(path = %path.display(), error = %e, "skipping a voice log");
            None
        }) else {
            continue;
        };

        let changes = relabel(&log, book);
        if changes.is_empty() {
            continue;
        }
        apply(&mut log, book, &changes);
        log.save(&path)?;
        out.push((log.meeting.clone(), changes));
    }

    out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Ok(out)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 0.0;
    }
    dot / (na * nb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const NGOC: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const BINH: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const KHACH: [f32; 4] = [0.0, 0.0, 1.0, 0.0];

    fn near(v: [f32; 4], jitter: f32) -> Vec<f32> {
        vec![v[0] + jitter, v[1] + jitter / 2.0, v[2], v[3]]
    }

    fn log() -> VoiceLog {
        VoiceLog::new(MeetingId::from("01A".to_string()), "campplus-sv")
    }

    #[test]
    fn a_known_voice_gets_its_name_immediately() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()]).unwrap();

        let mut attributor = Attributor::new(book, log());
        let label = attributor.attribute(0, 0.0, 3.0, &near(NGOC, 0.02));
        assert_eq!(label.as_ref().map(SpeakerId::as_str), Some("Ngọc"));
        assert_eq!(attributor.log().samples[0].person.as_deref(), Some("ngoc"));
    }

    #[test]
    fn unknown_voices_are_numbered_and_stay_apart() {
        let mut attributor = Attributor::new(VoiceBook::default(), log());
        assert_eq!(
            attributor.attribute(0, 0.0, 3.0, &NGOC).unwrap().as_str(),
            "Người 1"
        );
        assert_eq!(
            attributor.attribute(1, 4.0, 3.0, &BINH).unwrap().as_str(),
            "Người 2"
        );
        // The first voice returning is the same person, not a third.
        assert_eq!(
            attributor.attribute(2, 8.0, 3.0, &near(NGOC, 0.05)).unwrap().as_str(),
            "Người 1"
        );
    }

    #[test]
    fn an_utterance_too_short_to_embed_gets_no_label_rather_than_a_guess() {
        let mut attributor = Attributor::new(VoiceBook::default(), log());
        assert_eq!(attributor.attribute(0, 0.0, 0.3, &NGOC), None);
        assert!(attributor.log().samples.is_empty(), "a guess was still stored");
    }

    #[test]
    fn naming_one_voice_fixes_every_line_it_said() {
        // The whole point of keeping vectors: correction is a loop and a dot product.
        let mut attributor = Attributor::new(VoiceBook::default(), log());
        attributor.attribute(0, 0.0, 3.0, &NGOC);
        attributor.attribute(1, 4.0, 3.0, &BINH);
        attributor.attribute(2, 8.0, 3.0, &near(NGOC, 0.03));
        let mut log = attributor.into_log();

        let label = SpeakerId::from("Người 1".to_string());
        let mut book = VoiceBook::default();
        let id = book.enroll("Ngọc", &log.embeddings_for(&label)).unwrap();
        confirm(&mut log, &label, &id, "Ngọc");

        let named: Vec<&str> = log.samples.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(named, vec!["Ngọc", "Người 2", "Ngọc"]);
        assert!(log.samples[0].confirmed);
    }

    #[test]
    fn a_confirmed_line_is_never_overturned_by_a_later_match() {
        // A user answered. A model that disagrees afterwards is the thing that is wrong.
        let mut log = log();
        log.samples.push(VoiceSample {
            seq: 0,
            t0: 0.0,
            duration: 3.0,
            embedding: NGOC.to_vec(),
            person: Some("binh".into()),
            label: SpeakerId::from("Bình".to_string()),
            confirmed: true,
        });

        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()]).unwrap();

        assert!(relabel(&log, &book).is_empty());
    }

    #[test]
    fn an_unnamed_voice_in_an_old_meeting_gets_named_retroactively() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path().to_path_buf();

        // Two meetings recorded before anyone was named.
        for (n, embedding) in [("01A", NGOC), ("01B", NGOC)] {
            let mut log = VoiceLog::new(MeetingId::from(n.to_string()), "campplus-sv");
            log.samples.push(VoiceSample {
                seq: 0,
                t0: 0.0,
                duration: 3.0,
                embedding: near(embedding, 0.02),
                person: None,
                label: SpeakerId::from("Người 1".to_string()),
                confirmed: false,
            });
            log.save(&VoiceLog::path_for(&voices, &log.meeting)).unwrap();
        }

        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()]).unwrap();

        let swept = resweep(&voices, &book).unwrap();
        assert_eq!(swept.len(), 2, "both meetings should have been corrected");
        assert_eq!(swept[0].1[0].to.as_str(), "Ngọc");

        // And it persisted, so the next sweep has nothing left to do.
        assert!(resweep(&voices, &book).unwrap().is_empty());
    }

    #[test]
    fn a_sweep_leaves_alone_voices_that_are_still_nobody() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path().to_path_buf();

        let mut log = log();
        log.samples.push(VoiceSample {
            seq: 0,
            t0: 0.0,
            duration: 3.0,
            embedding: KHACH.to_vec(),
            person: None,
            label: SpeakerId::from("Người 1".to_string()),
            confirmed: false,
        });
        log.save(&VoiceLog::path_for(&voices, &log.meeting)).unwrap();

        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()]).unwrap();

        assert!(resweep(&voices, &book).unwrap().is_empty());
    }

    #[test]
    fn a_meeting_with_no_history_sweeps_without_complaining() {
        let dir = TempDir::new().unwrap();
        assert!(resweep(dir.path(), &VoiceBook::default()).unwrap().is_empty());
    }

    #[test]
    fn vectors_outlive_the_audio_they_came_from() {
        // Retention deletes recordings after a month; the vectors have to survive that or a name
        // applied next year cannot fix last year.
        let dir = TempDir::new().unwrap();
        let path = VoiceLog::path_for(dir.path(), &MeetingId::from("01A".to_string()));
        assert!(
            !path.starts_with(dir.path().join("audio")),
            "vectors were stored under the audio directory: {}",
            path.display()
        );
    }

    #[test]
    fn a_log_from_a_newer_build_is_refused_rather_than_misread() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.json");
        std::fs::write(
            &path,
            r#"{"meeting":"01A","schema":99,"model":"future","samples":[]}"#,
        )
        .unwrap();
        assert!(VoiceLog::load(&path).is_err());
    }

    #[test]
    fn a_log_that_was_never_written_is_absence_not_failure() {
        let dir = TempDir::new().unwrap();
        assert!(VoiceLog::load(&dir.path().join("nothing.json")).unwrap().is_none());
    }
}
