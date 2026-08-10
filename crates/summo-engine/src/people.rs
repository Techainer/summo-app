//! Who Summo thinks was speaking, and letting the user say otherwise.
//!
//! Diarization is guessing. It guesses well, and it still gets people wrong — two colleagues with
//! similar voices, somebody on a bad connection, a person who joins late and sounds different
//! through their laptop speakers than their headset. So the interesting part of this feature is not
//! the recognition, it is the correction: the user points at a name, and everything the system
//! believed on that basis has to be revised, including the meetings it already wrote.
//!
//! This module is the HTTP shape of `summo_diar`'s voice book. It owns no logic of its own beyond
//! reading and writing files under `~/.summo/voices/` — the arithmetic lives in
//! [`summo_diar::voices`] and [`summo_diar::attribution`], where it is tested without a server.
//!
//! Every operation here is a whole-file read-modify-write under a lock held by the caller's request.
//! That is fine at this size: a voice book is a few hundred kilobytes and corrections happen at
//! human speed, a handful per meeting at most.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, SpeakerId};
use summo_diar::{VoiceBook, VoiceLog, attribution};

/// File holding the people Summo can recognise.
fn book_path(voices_dir: &Path) -> PathBuf {
    voices_dir.join("book.json")
}

/// A person as the interface needs them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonView {
    pub id: String,
    pub name: String,
    /// Path relative to the vault, if the user picked a picture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// Utterances that shaped this profile.
    pub samples: usize,
    /// Of those, how many a human confirmed rather than the model guessed.
    ///
    /// Shown because it is the difference between "Summo thinks this is Ngọc" and "you told Summo
    /// this is Ngọc", and the user is entitled to know which one they are looking at.
    pub confirmed: usize,
    /// Distinct ways this voice has sounded — headset, laptop mic, phone.
    pub centroids: usize,
}

impl From<&summo_diar::Person> for PersonView {
    fn from(p: &summo_diar::Person) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            avatar: p.avatar.clone(),
            samples: p.sample_count(),
            confirmed: p.confirmed_count(),
            centroids: p.centroids.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeopleView {
    pub people: Vec<PersonView>,
    /// The embedding model these profiles belong to, if any vectors are stored.
    ///
    /// Surfaced so the interface can explain a mismatch rather than showing an empty list after a
    /// model change. See `summo_diar::space`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

/// A voice in one meeting that has not been given a name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownVoice {
    /// The provisional label, e.g. `S2`.
    pub label: String,
    /// How many utterances it covers — a one-line interjection is not worth naming.
    pub utterances: usize,
    /// Seconds of speech, which is a better guide than utterance count.
    pub seconds: f64,
    /// Who the book thinks it might be, best first. Empty when nobody is close.
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub name: String,
    /// Cosine similarity, 0 to 1.
    pub similarity: f32,
}

/// What a correction changed, so the interface can say so rather than silently redrawing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionView {
    pub person: PersonView,
    /// Utterances relabelled in the meeting the user was looking at.
    pub relabelled_here: usize,
    /// Past meetings that changed, and by how much.
    pub relabelled_elsewhere: Vec<MeetingChange>,
    /// Profiles the correction took samples away from, because they had been wrong.
    pub corrected_profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingChange {
    pub meeting: String,
    pub utterances: usize,
}

/// Suggestions below this are noise, not candidates.
///
/// Deliberately lower than the automatic match threshold: this list is read by a human who can
/// reject a wrong guess in one glance, so it should err towards offering too much rather than
/// hiding the right answer.
const SUGGEST_FLOOR: f32 = 0.40;

/// Suggestions offered per unknown voice. More than three is a list, not a hint.
const MAX_SUGGESTIONS: usize = 3;

/// Everyone in the book.
pub fn list(voices_dir: &Path) -> Result<PeopleView> {
    let book = VoiceBook::load(book_path(voices_dir))?;
    Ok(PeopleView {
        people: book.people().map(PersonView::from).collect(),
        space: book.space().map(ToString::to_string),
    })
}

/// The unnamed voices in one meeting, with who they might be.
pub fn unknowns(voices_dir: &Path, meeting: &MeetingId) -> Result<Vec<UnknownVoice>> {
    let book = VoiceBook::load(book_path(voices_dir))?;
    let Some(log) = VoiceLog::load(&VoiceLog::path_for(voices_dir, meeting))? else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for label in log.labels() {
        // A label already attached to somebody is not a question for the user.
        let named = log
            .samples
            .iter()
            .any(|s| s.label == label && s.person.is_some());
        if named {
            continue;
        }
        let embeddings = log.embeddings_for(&label);
        if embeddings.is_empty() {
            continue;
        }
        let seconds: f64 = log
            .samples
            .iter()
            .filter(|s| s.label == label)
            .map(|s| s.duration)
            .sum();

        let mut suggestions: Vec<Suggestion> = book
            .people()
            .map(|p| Suggestion {
                id: p.id.clone(),
                name: p.name.clone(),
                // Score against the voice as a whole, not one utterance of it.
                similarity: embeddings
                    .iter()
                    .map(|e| p.similarity(e))
                    .fold(f32::MIN, f32::max),
            })
            .filter(|s| s.similarity >= SUGGEST_FLOOR)
            .collect();
        suggestions.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        suggestions.truncate(MAX_SUGGESTIONS);

        out.push(UnknownVoice {
            label: label.to_string(),
            utterances: embeddings.len(),
            seconds,
            suggestions,
        });
    }

    // Longest first: the voice that spoke most is the one worth naming.
    out.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    Ok(out)
}

/// Give a name to a voice in a meeting.
///
/// `name` may be somebody already in the book or somebody new; the book decides by folded name, so
/// "Ngọc" and "Ngoc" are the same person. This is the operation behind picking a name from a list in
/// the interface, and it does three things that have to happen together:
///
/// 1. attaches the voice to that person, taking the samples back off any profile they were wrongly
///    added to,
/// 2. relabels this meeting's log,
/// 3. re-sweeps past meetings, because a voice that was unknown last week may be recognisable now.
pub fn name_voice(
    voices_dir: &Path,
    meeting: &MeetingId,
    label: &str,
    name: &str,
) -> Result<CorrectionView> {
    let mut book = VoiceBook::load(book_path(voices_dir))?;
    let log_path = VoiceLog::path_for(voices_dir, meeting);
    let mut log = VoiceLog::load(&log_path)?
        .ok_or_else(|| Error::Other(format!("no voice log for meeting {meeting}")))?;

    let label = SpeakerId::from(label.to_string());
    let correction = attribution::correct(voices_dir, &mut book, &mut log, &label, name)?;

    log.save(&log_path)?;
    book.save()?;

    let person = book
        .get(&correction.person)
        .map(PersonView::from)
        .ok_or_else(|| Error::Other("correction produced no profile".into()))?;

    Ok(CorrectionView {
        person,
        relabelled_here: correction.in_this_meeting,
        relabelled_elsewhere: correction
            .swept
            .into_iter()
            // A meeting the sweep visited but did not change is not news.
            .filter(|(_, changes)| !changes.is_empty())
            .map(|(meeting, changes)| MeetingChange {
                meeting: meeting.to_string(),
                utterances: changes.len(),
            })
            .collect(),
        corrected_profiles: correction
            .reassignment
            .removed_from
            .into_iter()
            .map(|(id, _)| id)
            .collect(),
    })
}

/// Rename somebody. Their voice is unaffected; only the label changes.
pub fn rename(voices_dir: &Path, id: &str, name: &str) -> Result<PersonView> {
    let mut book = VoiceBook::load(book_path(voices_dir))?;
    book.rename(id, name)?;
    book.save()?;
    book.get(id)
        .map(PersonView::from)
        .ok_or_else(|| Error::Other(format!("no person with id {id}")))
}

/// Attach or clear a picture.
pub fn set_avatar(voices_dir: &Path, id: &str, avatar: Option<String>) -> Result<PersonView> {
    let mut book = VoiceBook::load(book_path(voices_dir))?;
    book.set_avatar(id, avatar)?;
    book.save()?;
    book.get(id)
        .map(PersonView::from)
        .ok_or_else(|| Error::Other(format!("no person with id {id}")))
}

/// Fold one profile into another, for when the same person was learned twice.
pub fn merge(voices_dir: &Path, from: &str, into: &str) -> Result<PersonView> {
    let mut book = VoiceBook::load(book_path(voices_dir))?;
    book.merge(from, into)?;
    book.save()?;
    book.get(into)
        .map(PersonView::from)
        .ok_or_else(|| Error::Other(format!("no person with id {into}")))
}

/// Forget somebody entirely.
///
/// Transcripts keep the name that was written at the time — this removes the ability to recognise
/// the voice again, not the record of what was said.
pub fn forget(voices_dir: &Path, id: &str) -> Result<bool> {
    let mut book = VoiceBook::load(book_path(voices_dir))?;
    let removed = book.forget(id);
    if removed {
        book.save()?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Two clearly different voices, plus one close to the first.
    const NGOC: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const BINH: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const NGOC_ON_A_PHONE: [f32; 4] = [0.70, 0.714, 0.0, 0.0];

    fn voices() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    fn seed_book(dir: &Path) -> VoiceBook {
        let mut book = VoiceBook::load(book_path(dir)).expect("load");
        book.enroll("Ngọc", &[NGOC.to_vec()], true).expect("enroll");
        book.save().expect("save");
        book
    }

    fn seed_log(dir: &Path, meeting: &MeetingId, label: &str, embedding: [f32; 4], seconds: f64) {
        let path = VoiceLog::path_for(dir, meeting);
        let mut log = VoiceLog::load(&path)
            .expect("load")
            .unwrap_or_else(|| VoiceLog::new(meeting.clone(), "campplus-sv"));
        log.samples.push(summo_diar::VoiceSample {
            seq: log.samples.len() as u64,
            t0: 0.0,
            duration: seconds,
            label: SpeakerId::from(label.to_string()),
            person: None,
            confirmed: false,
            embedding: embedding.to_vec(),
        });
        log.save(&path).expect("save");
    }

    #[test]
    fn an_empty_vault_has_nobody_rather_than_failing() {
        let dir = voices();
        let view = list(dir.path()).expect("list");
        assert!(view.people.is_empty());
        assert!(view.space.is_none());
    }

    #[test]
    fn a_person_carries_what_the_interface_needs_to_show() {
        let dir = voices();
        seed_book(dir.path());
        let view = list(dir.path()).expect("list");
        let ngoc = &view.people[0];
        assert_eq!(ngoc.name, "Ngọc");
        assert_eq!(ngoc.samples, 1);
        assert_eq!(ngoc.confirmed, 1, "the user said so, the model did not guess");
    }

    #[test]
    fn a_meeting_with_no_log_has_no_unknowns() {
        let dir = voices();
        let found = unknowns(dir.path(), &MeetingId::from(String::from("01NOPE"))).expect("unknowns");
        assert!(found.is_empty());
    }

    #[test]
    fn an_unnamed_voice_is_offered_with_who_it_might_be() {
        let dir = voices();
        seed_book(dir.path());
        let meeting = MeetingId::from(String::from("01A"));
        seed_log(dir.path(), &meeting, "S2", NGOC_ON_A_PHONE, 30.0);

        let found = unknowns(dir.path(), &meeting).expect("unknowns");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "S2");
        assert_eq!(found[0].utterances, 1);
        assert_eq!(
            found[0].suggestions.first().map(|s| s.name.as_str()),
            Some("Ngọc"),
            "a voice close to Ngọc should suggest Ngọc"
        );
    }

    #[test]
    fn a_voice_nothing_resembles_is_offered_without_a_guess() {
        let dir = voices();
        seed_book(dir.path());
        let meeting = MeetingId::from(String::from("01B"));
        seed_log(dir.path(), &meeting, "S3", BINH, 20.0);

        let found = unknowns(dir.path(), &meeting).expect("unknowns");
        assert_eq!(found.len(), 1);
        assert!(
            found[0].suggestions.is_empty(),
            "an unrelated voice must not be given somebody else's name"
        );
    }

    #[test]
    fn the_longest_voice_is_asked_about_first() {
        let dir = voices();
        let meeting = MeetingId::from(String::from("01C"));
        seed_log(dir.path(), &meeting, "S2", BINH, 5.0);
        seed_log(dir.path(), &meeting, "S3", NGOC, 90.0);

        let found = unknowns(dir.path(), &meeting).expect("unknowns");
        assert_eq!(found[0].label, "S3", "90 seconds outranks 5");
    }

    #[test]
    fn naming_a_voice_creates_the_person_and_relabels_the_meeting() {
        let dir = voices();
        let meeting = MeetingId::from(String::from("01D"));
        seed_log(dir.path(), &meeting, "S2", BINH, 40.0);

        let result = name_voice(dir.path(), &meeting, "S2", "Bình").expect("name");
        assert_eq!(result.person.name, "Bình");
        assert_eq!(result.relabelled_here, 1);

        // And the voice is no longer a question.
        assert!(unknowns(dir.path(), &meeting).expect("unknowns").is_empty());
        assert_eq!(list(dir.path()).expect("list").people.len(), 1);
    }

    #[test]
    fn naming_a_voice_as_somebody_known_adds_to_them_rather_than_duplicating() {
        let dir = voices();
        seed_book(dir.path());
        let meeting = MeetingId::from(String::from("01E"));
        seed_log(dir.path(), &meeting, "S2", NGOC_ON_A_PHONE, 30.0);

        let result = name_voice(dir.path(), &meeting, "S2", "Ngọc").expect("name");
        assert_eq!(result.person.id, "ngoc");
        assert_eq!(
            list(dir.path()).expect("list").people.len(),
            1,
            "one person, not two"
        );
        assert!(
            result.person.centroids >= 2,
            "a second way of sounding earns its own centroid"
        );
    }

    #[test]
    fn naming_a_voice_that_does_not_exist_is_an_error_not_a_silent_no_op() {
        let dir = voices();
        let meeting = MeetingId::from(String::from("01F"));
        seed_log(dir.path(), &meeting, "S2", BINH, 10.0);
        assert!(name_voice(dir.path(), &meeting, "S9", "Bình").is_err());
    }

    #[test]
    fn naming_in_a_meeting_with_no_log_is_an_error() {
        let dir = voices();
        assert!(name_voice(dir.path(), &MeetingId::from(String::from("01NOPE")), "S2", "Bình").is_err());
    }

    #[test]
    fn renaming_keeps_the_voice_and_changes_the_label() {
        let dir = voices();
        seed_book(dir.path());
        let renamed = rename(dir.path(), "ngoc", "Ngọc Nguyễn").expect("rename");
        assert_eq!(renamed.name, "Ngọc Nguyễn");
        assert_eq!(renamed.samples, 1, "the voice is unaffected");
    }

    #[test]
    fn renaming_somebody_who_is_not_there_is_an_error() {
        let dir = voices();
        assert!(rename(dir.path(), "nobody", "X").is_err());
    }

    #[test]
    fn an_avatar_can_be_set_and_cleared() {
        let dir = voices();
        seed_book(dir.path());
        let with = set_avatar(dir.path(), "ngoc", Some("attachments/ngoc.jpg".into())).expect("set");
        assert_eq!(with.avatar.as_deref(), Some("attachments/ngoc.jpg"));
        let without = set_avatar(dir.path(), "ngoc", None).expect("clear");
        assert!(without.avatar.is_none());
    }

    #[test]
    fn merging_folds_one_profile_into_another() {
        let dir = voices();
        let mut book = VoiceBook::load(book_path(dir.path())).expect("load");
        book.enroll("Ngọc", &[NGOC.to_vec()], true).expect("a");
        book.enroll("Ngoc B", &[NGOC_ON_A_PHONE.to_vec()], true)
            .expect("b");
        book.save().expect("save");

        let merged = merge(dir.path(), "ngoc-b", "ngoc").expect("merge");
        assert_eq!(merged.samples, 2);
        assert_eq!(list(dir.path()).expect("list").people.len(), 1);
    }

    #[test]
    fn forgetting_reports_whether_anything_was_there() {
        let dir = voices();
        seed_book(dir.path());
        assert!(forget(dir.path(), "ngoc").expect("forget"));
        assert!(!forget(dir.path(), "ngoc").expect("forget again"));
        assert!(list(dir.path()).expect("list").people.is_empty());
    }

    /// The case that motivated storing samples rather than a running average: a third person is
    /// recognised as somebody who already exists, and the human fixes it.
    #[test]
    fn correcting_a_false_match_takes_the_samples_back_off_the_wrong_person() {
        let dir = voices();
        let mut book = VoiceBook::load(book_path(dir.path())).expect("load");
        // Ngọc's profile has wrongly absorbed a voice that is not hers.
        book.enroll("Ngọc", &[NGOC.to_vec()], true).expect("real");
        book.enroll("Ngọc", &[BINH.to_vec()], false).expect("wrong");
        book.save().expect("save");
        assert_eq!(list(dir.path()).expect("list").people[0].samples, 2);

        let meeting = MeetingId::from(String::from("01G"));
        seed_log(dir.path(), &meeting, "S3", BINH, 25.0);
        let result = name_voice(dir.path(), &meeting, "S3", "Bình").expect("correct");

        assert_eq!(result.person.name, "Bình");
        assert!(
            result.corrected_profiles.contains(&"ngoc".to_string()),
            "the profile that was wrong should say so: {:?}",
            result.corrected_profiles
        );

        let people = list(dir.path()).expect("list").people;
        let ngoc = people.iter().find(|p| p.id == "ngoc").expect("still there");
        assert_eq!(
            ngoc.samples, 1,
            "the sample that was never hers must be gone"
        );
    }
}
