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

use crate::voices::{Match, Person, Reassignment, VoiceBook, unknown_speaker};

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

    /// Where a meeting's vectors live, in the binary format of ADR 0003.
    ///
    /// Under `voices/`, not next to the audio: retention deletes recordings after a month, and
    /// these have to outlive that — they are what lets a name applied next year fix last year's
    /// transcripts.
    #[must_use]
    pub fn path_for(voices_dir: &Path, meeting: &MeetingId) -> PathBuf {
        voices_dir
            .join("meetings")
            .join(format!("{}.vec", meeting.as_str()))
    }

    /// The JSON path a previous release wrote.
    ///
    /// Kept for one release so an existing vault is read rather than silently starting empty —
    /// losing these vectors would cost every past correction.
    ///
    /// Public so a migration tool and the tests can name the file the old release wrote; `load`
    /// finds it on its own.
    #[must_use]
    pub fn legacy_path_for(voices_dir: &Path, meeting: &MeetingId) -> PathBuf {
        voices_dir
            .join("meetings")
            .join(format!("{}.json", meeting.as_str()))
    }

    /// Read a log, in either format.
    ///
    /// Dispatches on the file's magic rather than its extension, so a `.vec` that is really JSON —
    /// which is what a half-finished migration leaves behind — still reads.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Fall back to the JSON a previous release wrote next to it.
                let legacy = path.with_extension("json");
                if legacy == path {
                    return Ok(None);
                }
                match std::fs::read(&legacy) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(Error::io(&legacy, e)),
                }
            }
            Err(e) => return Err(Error::io(path, e)),
        };

        if crate::vecfile::is_binary(&bytes) {
            return Self::from_binary(&bytes, path).map(Some);
        }
        Self::from_json(&bytes, path).map(Some)
    }

    fn from_binary(bytes: &[u8], path: &Path) -> Result<Self> {
        let (header, records) = crate::vecfile::read(bytes)
            .map_err(|e| Error::Other(format!("cannot read {}: {e}", path.display())))?;
        Ok(Self {
            meeting: MeetingId::from(header.meeting),
            schema: SCHEMA,
            model: header.model,
            samples: records
                .into_iter()
                .map(|r| VoiceSample {
                    seq: r.seq,
                    t0: r.t0,
                    duration: r.duration,
                    embedding: r.embedding,
                    person: r.person,
                    label: SpeakerId::from(r.label),
                    confirmed: r.confirmed,
                })
                .collect(),
        })
    }

    fn from_json(bytes: &[u8], path: &Path) -> Result<Self> {
        let log: Self = serde_json::from_slice(bytes)
            .map_err(|e| Error::Other(format!("cannot parse {}: {e}", path.display())))?;
        if log.schema > SCHEMA {
            return Err(Error::Other(format!(
                "{} was written by a newer build",
                path.display()
            )));
        }
        Ok(log)
    }

    /// Write the log, and clear away the JSON a previous release left.
    ///
    /// Removing the legacy file is what makes the migration finish: leaving both would mean a later
    /// load could pick up stale vectors if the binary one were ever lost.
    pub fn save(&self, path: &Path) -> Result<()> {
        let dims = self.samples.first().map_or(1, |s| s.embedding.len());
        let records: Vec<crate::vecfile::Record> = self
            .samples
            .iter()
            .map(|s| crate::vecfile::Record {
                seq: s.seq,
                t0: s.t0,
                duration: s.duration,
                confirmed: s.confirmed,
                label: s.label.to_string(),
                person: s.person.clone(),
                embedding: s.embedding.clone(),
            })
            .collect();

        let header = crate::vecfile::Header {
            dims,
            count: records.len(),
            model: self.model.clone(),
            // Revision is not tracked on the log itself yet; the book is where a space is claimed.
            revision: String::new(),
            meeting: self.meeting.to_string(),
        };
        let bytes = crate::vecfile::write(&header, &records)?;
        crate::vecfile::write_atomically(path, &bytes)?;

        let legacy = path.with_extension("json");
        if legacy != path {
            std::fs::remove_file(&legacy).ok();
        }
        Ok(())
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
    ///
    /// Full profiles rather than one vector each, for the same reason named people get several
    /// centroids: an unnamed voice varies exactly as much as a named one does, and a single
    /// drifting average loses that spread precisely when it is still being established.
    unknown: Vec<Person>,
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
                let name = self.book.name_of(&person);
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
            .map(|(i, p)| (i, p.similarity(embedding)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        if let Some((i, similarity)) = best
            && similarity >= self.same_voice
        {
            self.unknown[i].absorb(embedding, false);
            return i;
        }
        let n = self.unknown.len() + 1;
        let mut person = Person::new(format!("unknown-{n}"), unknown_speaker(n).as_str().to_string());
        person.absorb(embedding, false);
        self.unknown.push(person);
        self.unknown.len() - 1
    }

    /// The provisional profiles of this meeting's unnamed voices.
    #[must_use]
    pub fn unknown_voices(&self) -> &[Person] {
        &self.unknown
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
            let to = SpeakerId::from(book.name_of(&person));
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
pub fn confirm(log: &mut VoiceLog, label: &SpeakerId, person_id: &str, name: &str) -> usize {
    let to = SpeakerId::from(name.to_string());
    let mut count = 0;
    for sample in log.samples.iter_mut().filter(|s| &s.label == label) {
        sample.label = to.clone();
        sample.person = Some(person_id.to_string());
        sample.confirmed = true;
        count += 1;
    }
    count
}

/// Everything one correction changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Correction {
    /// The person the utterances now belong to.
    pub person: String,
    /// Which profiles gave samples up, and which disappeared entirely.
    pub reassignment: Reassignment,
    /// Lines fixed in the meeting the user was looking at.
    pub in_this_meeting: usize,
    /// Lines fixed in every other meeting on disk.
    pub swept: Vec<(MeetingId, Vec<Relabel>)>,
}

/// A user says the utterances labelled `label` are `name`. Do all of it.
///
/// This is the whole correction in one call, because doing only part of it is what makes voice
/// recognition feel broken:
///
/// 1. Move the samples to the right person, **taking them out of the profile that wrongly claimed
///    them**. Without the removal, the next meeting repeats the mistake.
/// 2. Mark this meeting's lines as confirmed, so no later match overturns a human.
/// 3. Sweep every other meeting, because the same voice was probably misattributed there too.
///
/// Step three is affordable only because the vectors were kept: it is a loop and a dot product over
/// stored numbers, not a re-run of anything.
pub fn correct(
    voices_dir: &Path,
    book: &mut VoiceBook,
    log: &mut VoiceLog,
    label: &SpeakerId,
    name: &str,
) -> Result<Correction> {
    let embeddings = log.embeddings_for(label);
    if embeddings.is_empty() {
        return Err(Error::Other(format!(
            "no utterances in this meeting are labelled {}",
            label.as_str()
        )));
    }

    let reassignment = book.reassign(name, &embeddings)?;
    book.save()?;

    let in_this_meeting = confirm(log, label, &reassignment.person, name);
    log.save(&VoiceLog::path_for(voices_dir, &log.meeting))?;

    // The current meeting is already correct on disk, so the sweep finds nothing left to do in it
    // and reports only the others.
    let swept = resweep(voices_dir, book)?;

    Ok(Correction {
        person: reassignment.person.clone(),
        reassignment,
        in_this_meeting,
        swept,
    })
}

/// Relabel every meeting on disk against the current book.
///
/// This is the sweep that runs after somebody names a voice: every past `Người 2` that is really
/// them becomes their name. Reported per meeting so a caller can rewrite only the files that
/// changed.
pub fn resweep(voices_dir: &Path, book: &VoiceBook) -> Result<Vec<(MeetingId, Vec<Relabel>)>> {
    let dir = voices_dir.join("meetings");
    let mut out = Vec::new();

    // One entry per meeting, preferring the binary file when a vault is mid-migration and both
    // exist — otherwise the same meeting would be swept twice and reported twice.
    let mut candidates: std::collections::BTreeMap<std::ffi::OsString, PathBuf> =
        std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let binary = match path.extension().and_then(|e| e.to_str()) {
            Some("vec") => true,
            Some("json") => false,
            _ => continue,
        };
        let Some(stem) = path.file_stem().map(ToOwned::to_owned) else {
            continue;
        };
        if binary || !candidates.contains_key(&stem) {
            candidates.insert(stem, path);
        }
    }

    for path in candidates.into_values() {
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

    /// A vault written by the previous release must keep its vectors.
    ///
    /// Losing them would cost every past correction: the names in old transcripts would survive,
    /// but Summo would no longer be able to fix them when somebody is renamed.
    #[test]
    fn a_json_log_from_a_previous_release_is_still_read() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path();
        let meeting = MeetingId::from("01A".to_string());

        let mut original = log();
        original.samples.push(VoiceSample {
            seq: 0,
            t0: 1.0,
            duration: 4.0,
            embedding: NGOC.to_vec(),
            person: Some("ngoc".into()),
            label: SpeakerId::from("Ngọc".to_string()),
            confirmed: true,
        });

        // Exactly what the old code wrote: JSON, at the `.json` path.
        let legacy = VoiceLog::legacy_path_for(voices, &meeting);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, serde_json::to_vec(&original).unwrap()).unwrap();

        let loaded = VoiceLog::load(&VoiceLog::path_for(voices, &meeting))
            .expect("load")
            .expect("the old file must be found");
        assert_eq!(loaded.samples.len(), 1);
        assert_eq!(loaded.samples[0].embedding, NGOC.to_vec());
        assert!(loaded.samples[0].confirmed);
    }

    #[test]
    fn saving_migrates_the_old_file_away() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path();
        let meeting = MeetingId::from("01A".to_string());

        let mut original = log();
        original.samples.push(VoiceSample {
            seq: 0,
            t0: 0.0,
            duration: 2.0,
            embedding: BINH.to_vec(),
            person: None,
            label: SpeakerId::from("S2".to_string()),
            confirmed: false,
        });
        let legacy = VoiceLog::legacy_path_for(voices, &meeting);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, serde_json::to_vec(&original).unwrap()).unwrap();

        let path = VoiceLog::path_for(voices, &meeting);
        let loaded = VoiceLog::load(&path).unwrap().unwrap();
        loaded.save(&path).unwrap();

        assert!(path.exists(), "the binary file must be written");
        assert!(
            !legacy.exists(),
            "the old file must go, or a later load could resurrect stale vectors"
        );

        // And the migrated file still says the same thing.
        let back = VoiceLog::load(&path).unwrap().unwrap();
        assert_eq!(back.samples[0].embedding, BINH.to_vec());
        assert_eq!(back.meeting.as_str(), "01A");
        assert_eq!(back.model, "campplus-sv");
    }

    #[test]
    fn a_resweep_covers_both_formats_and_counts_each_meeting_once() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path();
        let meetings = voices.join("meetings");
        std::fs::create_dir_all(&meetings).unwrap();

        // One meeting still in JSON, one already migrated.
        for (id, binary) in [("01OLD", false), ("01NEW", true)] {
            let meeting = MeetingId::from(id.to_string());
            let mut log = VoiceLog::new(meeting.clone(), "campplus-sv");
            log.samples.push(VoiceSample {
                seq: 0,
                t0: 0.0,
                duration: 5.0,
                embedding: NGOC.to_vec(),
                person: None,
                label: SpeakerId::from("S2".to_string()),
                confirmed: false,
            });
            if binary {
                log.save(&VoiceLog::path_for(voices, &meeting)).unwrap();
            } else {
                std::fs::write(
                    VoiceLog::legacy_path_for(voices, &meeting),
                    serde_json::to_vec(&log).unwrap(),
                )
                .unwrap();
            }
        }

        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

        let swept = resweep(voices, &book).unwrap();
        assert_eq!(swept.len(), 2, "both formats must be swept: {swept:?}");
        let ids: Vec<&str> = swept.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(ids, vec!["01NEW", "01OLD"]);
    }

    #[test]
    fn a_half_migrated_meeting_is_swept_once_not_twice() {
        let dir = TempDir::new().unwrap();
        let voices = dir.path();
        let meeting = MeetingId::from("01A".to_string());

        let mut log = VoiceLog::new(meeting.clone(), "campplus-sv");
        log.samples.push(VoiceSample {
            seq: 0,
            t0: 0.0,
            duration: 5.0,
            embedding: NGOC.to_vec(),
            person: None,
            label: SpeakerId::from("S2".to_string()),
            confirmed: false,
        });
        // Both files present, as an interrupted migration would leave them.
        log.save(&VoiceLog::path_for(voices, &meeting)).unwrap();
        std::fs::write(
            VoiceLog::legacy_path_for(voices, &meeting),
            serde_json::to_vec(&log).unwrap(),
        )
        .unwrap();

        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

        let swept = resweep(voices, &book).unwrap();
        assert_eq!(swept.len(), 1, "one meeting, one entry: {swept:?}");
    }

    #[test]
    fn a_known_voice_gets_its_name_immediately() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

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
        let id = book.enroll("Ngọc", &log.embeddings_for(&label), true).unwrap();
        confirm(&mut log, &label, &id, "Ngọc");

        let named: Vec<&str> = log.samples.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(named, vec!["Ngọc", "Người 2", "Ngọc"]);
        assert!(log.samples[0].confirmed);
    }

    #[test]
    fn an_unnamed_voice_keeps_its_own_spread() {
        // The same reason a named person gets several centroids: an unnamed voice varies just as
        // much, and it is still being established, so flattening it is worst exactly here.
        // Cosine ≈ 0.70 with NGOC: the same voice by the within-meeting threshold, but a
        // different enough recording that it deserves its own centroid.
        let other_take = vec![0.70, 0.714, 0.0, 0.0];
        let mut attributor = Attributor::new(VoiceBook::default(), log());
        attributor.attribute(0, 0.0, 3.0, &NGOC);
        attributor.attribute(1, 4.0, 3.0, &other_take);

        let unknown = attributor.unknown_voices();
        assert_eq!(unknown.len(), 1, "two takes on one voice became two people");
        assert!(
            unknown[0].centroids.len() >= 2,
            "the spread was flattened into one centroid"
        );
        assert_eq!(unknown[0].sample_count(), 2);
    }

    #[test]
    fn correcting_a_wrong_match_fixes_this_meeting_the_book_and_the_history() {
        // The case the user runs into: a third person is recognised as Ngọc. Fixing it has to do
        // all three things, or the mistake comes straight back next meeting.
        let dir = TempDir::new().unwrap();
        let voices = dir.path().to_path_buf();

        let mut book = VoiceBook::load(voices.join("voices.json")).unwrap();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        // Ngọc's profile picks up a stranger by mistake.
        book.enroll("Ngọc", &[KHACH.to_vec()], false).unwrap();

        // An older meeting where the same stranger was already called Ngọc.
        let mut old = VoiceLog::new(MeetingId::from("00Z".to_string()), "campplus-sv");
        old.samples.push(VoiceSample {
            seq: 0,
            t0: 0.0,
            duration: 3.0,
            embedding: near(KHACH, 0.01),
            person: Some("ngoc".into()),
            label: SpeakerId::from("Ngọc".to_string()),
            confirmed: false,
        });
        old.save(&VoiceLog::path_for(&voices, &old.meeting)).unwrap();

        // Today's meeting, where the user notices.
        let mut today = log();
        let mut attributor = Attributor::new(book.clone(), today.clone());
        attributor.attribute(0, 0.0, 3.0, &near(KHACH, 0.02));
        today = attributor.into_log();
        assert_eq!(today.samples[0].label.as_str(), "Ngọc", "setup failed");
        today.save(&VoiceLog::path_for(&voices, &today.meeting)).unwrap();

        let done = correct(
            &voices,
            &mut book,
            &mut today,
            &SpeakerId::from("Ngọc".to_string()),
            "Bình",
        )
        .unwrap();

        assert_eq!(done.person, "binh");
        assert_eq!(done.in_this_meeting, 1);
        // The stranger's sample left Ngọc, so the mistake cannot repeat.
        assert_eq!(done.reassignment.removed_from, vec![("ngoc".to_string(), 1)]);
        assert_eq!(book.identify(&KHACH).person(), Some("binh"));
        assert_eq!(book.identify(&NGOC).person(), Some("ngoc"), "Ngọc was damaged");
        // And the old meeting was fixed without anything being re-run.
        assert_eq!(done.swept.len(), 1);
        assert_eq!(done.swept[0].0.as_str(), "00Z");
        assert_eq!(done.swept[0].1[0].to.as_str(), "Bình");
    }

    #[test]
    fn correcting_a_label_that_is_not_in_the_meeting_is_refused() {
        let dir = TempDir::new().unwrap();
        let mut book = VoiceBook::default();
        let mut log = log();
        assert!(
            correct(
                dir.path(),
                &mut book,
                &mut log,
                &SpeakerId::from("Người 9".to_string()),
                "Bình"
            )
            .is_err()
        );
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
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

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
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

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
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

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
