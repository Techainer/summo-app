//! The voice book: who Summo has learned to recognise, kept between meetings.
//!
//! [`crate::cluster`] separates voices *within* one meeting and calls them `S1`, `S2`. This is the
//! layer above: it remembers that the voice called `S2` last Tuesday belongs to Ngọc, so that the
//! next meeting says "Ngọc" without being asked again.
//!
//! Two decisions shape everything here.
//!
//! **A person is several centroids, not one.** The same voice over a headset, a phone and a laptop
//! microphone lands in noticeably different places, and averaging them produces a centroid that
//! matches none of them well. Each enrolment that is not already close to an existing centroid
//! becomes a new one, up to a cap.
//!
//! **Recognition never overwrites a human.** A name a user typed is the truth; matching only ever
//! proposes. When the two disagree, the user's answer is stored as another sample of that voice —
//! which is exactly the case where the model most needs the correction.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, SpeakerId};

/// Centroids kept per person.
///
/// Enough for the handful of devices one person actually uses; beyond that the two closest are
/// merged, which loses nothing that was not nearly duplicated.
const MAX_CENTROIDS: usize = 8;

/// How confidently a stored voice has to match before a name is used.
///
/// Higher than the within-meeting threshold on purpose: calling someone by the wrong *name* across
/// meetings is a worse failure than leaving two utterances in the same meeting unmerged.
pub const MATCH_THRESHOLD: f32 = 0.68;

/// Below this, the voice is confidently nobody known.
pub const REJECT_THRESHOLD: f32 = 0.52;

/// A person Summo can recognise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// Stable key, used in file names and wikilinks.
    pub id: String,
    pub name: String,
    /// One per distinct-sounding recording condition.
    pub centroids: Vec<Vec<f32>>,
    /// Utterances that went into this profile, for showing how well established it is.
    #[serde(default)]
    pub samples: u32,
    /// Path relative to the vault, e.g. `attachments/ngoc.jpg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

impl Person {
    /// Best similarity between `embedding` and any of this person's centroids.
    #[must_use]
    pub fn similarity(&self, embedding: &[f32]) -> f32 {
        self.centroids
            .iter()
            .map(|c| cosine(c, embedding))
            .fold(f32::MIN, f32::max)
            .max(0.0)
    }

    /// Fold an utterance into the profile.
    fn absorb(&mut self, embedding: &[f32]) {
        let embedding = normalize(embedding);
        self.samples += 1;

        // Close to a centroid already: nudge it rather than adding a near-duplicate.
        if let Some((best, similarity)) = self
            .centroids
            .iter_mut()
            .map(|c| {
                let s = cosine(c, &embedding);
                (c, s)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            && similarity >= MATCH_THRESHOLD
        {
            for (slot, sample) in best.iter_mut().zip(&embedding) {
                *slot = *slot * 0.85 + sample * 0.15;
            }
            let renormalized = normalize(best);
            best.copy_from_slice(&renormalized);
            return;
        }

        self.centroids.push(embedding);
        if self.centroids.len() > MAX_CENTROIDS {
            self.merge_closest_pair();
        }
    }

    /// Collapse the two most similar centroids, which is the pair that carries the least
    /// information about how many different ways this voice sounds.
    fn merge_closest_pair(&mut self) {
        let mut best = (0, 1, f32::MIN);
        for i in 0..self.centroids.len() {
            for j in (i + 1)..self.centroids.len() {
                let s = cosine(&self.centroids[i], &self.centroids[j]);
                if s > best.2 {
                    best = (i, j, s);
                }
            }
        }
        let (i, j, _) = best;
        let merged = normalize(
            &self.centroids[i]
                .iter()
                .zip(&self.centroids[j])
                .map(|(a, b)| (a + b) / 2.0)
                .collect::<Vec<f32>>(),
        );
        self.centroids[i] = merged;
        self.centroids.remove(j);
    }
}

/// What the book concluded about a voice.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Confidently a person already known.
    Known { person: String, similarity: f32 },
    /// Similar to someone, but not enough to put their name on it.
    ///
    /// Deliberately not a label: showing the wrong name is worse than showing none, because a
    /// reader trusts a name and does not check it.
    Unsure { person: String, similarity: f32 },
    /// Nobody known sounds like this.
    Unknown { best: f32 },
}

impl Match {
    /// The name to display, if any.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Known { person, .. } => Some(person),
            Self::Unsure { .. } | Self::Unknown { .. } => None,
        }
    }
}

/// Everyone Summo can recognise.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceBook {
    #[serde(default)]
    people: BTreeMap<String, Person>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl VoiceBook {
    /// Read the book, treating a missing file as an empty one.
    ///
    /// A first run has met nobody yet, and that is not an error worth stopping a meeting for.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut book = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Self>(&text)
                .map_err(|e| Error::Other(format!("cannot parse {}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        book.path = Some(path);
        Ok(book)
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        // Through a rename: a half-written voice book read at the start of the next meeting would
        // lose every profile at once.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(|e| Error::io(&temporary, e))?;
        std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))
    }

    pub fn people(&self) -> impl Iterator<Item = &Person> {
        self.people.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.people.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.people.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Person> {
        self.people.get(id)
    }

    /// Who this voice sounds like.
    #[must_use]
    pub fn identify(&self, embedding: &[f32]) -> Match {
        let best = self
            .people
            .values()
            .map(|p| (p.id.clone(), p.similarity(embedding)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        match best {
            Some((person, similarity)) if similarity >= MATCH_THRESHOLD => {
                Match::Known { person, similarity }
            }
            Some((person, similarity)) if similarity >= REJECT_THRESHOLD => {
                Match::Unsure { person, similarity }
            }
            Some((_, similarity)) => Match::Unknown { best: similarity },
            None => Match::Unknown { best: 0.0 },
        }
    }

    /// Teach the book that these utterances are `name`, returning the person's id.
    ///
    /// This is what a user choosing a name in the interface calls. It is additive: naming a voice
    /// the model already matched to someone else does not move it, it adds a sample — the model was
    /// wrong about a voice it will hear again, and that is precisely what it needs to learn from.
    pub fn enroll(&mut self, name: &str, embeddings: &[Vec<f32>]) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Other("a person needs a name".into()));
        }
        let id = slug(name);

        let person = self.people.entry(id.clone()).or_insert_with(|| Person {
            id: id.clone(),
            name: name.to_string(),
            centroids: Vec::new(),
            samples: 0,
            avatar: None,
        });
        // A rename keeps the same id, so links and history survive being corrected.
        person.name = name.to_string();
        for embedding in embeddings {
            person.absorb(embedding);
        }
        Ok(id)
    }

    /// Attach a picture, so the transcript can show a face rather than a letter.
    pub fn set_avatar(&mut self, id: &str, avatar: Option<String>) -> Result<()> {
        self.people
            .get_mut(id)
            .ok_or_else(|| Error::Other(format!("no person with id {id}")))?
            .avatar = avatar;
        Ok(())
    }

    /// Fold one person into another, for when two profiles turn out to be the same voice.
    pub fn merge(&mut self, from: &str, into: &str) -> Result<()> {
        if from == into {
            return Ok(());
        }
        let source = self
            .people
            .remove(from)
            .ok_or_else(|| Error::Other(format!("no person with id {from}")))?;
        let target = self
            .people
            .get_mut(into)
            .ok_or_else(|| Error::Other(format!("no person with id {into}")))?;
        // Absorbing bumps the sample count per centroid, but a merge moves people, not utterances;
        // the count is restored so it keeps meaning "utterances heard".
        let before = target.samples;
        for centroid in &source.centroids {
            target.absorb(centroid);
        }
        target.samples = before + source.samples;
        Ok(())
    }

    pub fn forget(&mut self, id: &str) -> bool {
        self.people.remove(id).is_some()
    }
}

/// A file-name-safe key derived from a name.
fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true;
    for ch in name.chars().flat_map(char::to_lowercase) {
        let folded = fold(ch);
        if folded.is_alphanumeric() {
            out.push(folded);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed
    }
}

/// Vietnamese diacritics to their base letters, so `Ngọc` and `ngoc` are one person.
fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ' | 'ặ'
        | 'ẳ' | 'ẵ' => 'a',
        'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' => 'e',
        'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' => 'i',
        'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ' | 'ợ'
        | 'ở' | 'ỡ' => 'o',
        'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' => 'u',
        'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'đ' => 'd',
        other => other,
    }
}

/// Label for a voice nobody has named yet.
#[must_use]
pub fn unknown_speaker(n: usize) -> SpeakerId {
    SpeakerId::from(format!("Người {n}"))
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
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

    /// A voice as a direction in space, plus a little variation.
    fn voice(seed: f32, jitter: f32) -> Vec<f32> {
        (0..16)
            .map(|i| (i as f32 * seed).sin() + jitter * ((i * 7) as f32).cos())
            .collect()
    }

    #[test]
    fn a_named_voice_is_recognised_next_time() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[voice(1.0, 0.0)]).unwrap();

        let m = book.identify(&voice(1.0, 0.02));
        assert!(matches!(&m, Match::Known { person, .. } if person == "ngoc"), "got {m:?}");
        assert_eq!(m.name(), Some("ngoc"));
    }

    #[test]
    fn a_voice_nobody_knows_is_not_given_someone_elses_name() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[voice(1.0, 0.0)]).unwrap();

        let m = book.identify(&voice(5.0, 0.0));
        assert!(matches!(m, Match::Unknown { .. }), "got {m:?}");
        assert_eq!(m.name(), None);
    }

    #[test]
    fn a_borderline_voice_is_left_unnamed_rather_than_guessed() {
        // Showing the wrong name is worse than showing none: a reader trusts a name and does not
        // go back to the audio to check it.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[vec![1.0, 0.0, 0.0, 0.0]]).unwrap();

        // Cosine 0.6 — between the reject and match thresholds.
        let m = book.identify(&[0.6, 0.8, 0.0, 0.0]);
        assert!(matches!(m, Match::Unsure { .. }), "got {m:?}");
        assert_eq!(m.name(), None);
    }

    #[test]
    fn names_that_differ_only_by_tone_marks_are_the_same_person() {
        let mut book = VoiceBook::default();
        let a = book.enroll("Ngọc", &[voice(1.0, 0.0)]).unwrap();
        let b = book.enroll("ngoc", &[voice(1.0, 0.01)]).unwrap();
        assert_eq!(a, b);
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn one_person_can_sound_like_several_devices() {
        // A headset and a phone put the same voice in different places. Averaging them would give a
        // centroid that matches neither.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[vec![1.0, 0.0, 0.0, 0.0]]).unwrap();
        book.enroll("Ngọc", &[vec![0.0, 1.0, 0.0, 0.0]]).unwrap();

        assert_eq!(book.get("ngoc").unwrap().centroids.len(), 2);
        assert!(matches!(book.identify(&[1.0, 0.05, 0.0, 0.0]), Match::Known { .. }));
        assert!(matches!(book.identify(&[0.05, 1.0, 0.0, 0.0]), Match::Known { .. }));
    }

    #[test]
    fn a_profile_does_not_grow_without_bound() {
        let mut book = VoiceBook::default();
        for i in 0..40 {
            book.enroll("Ngọc", &[voice(i as f32 * 0.9 + 0.3, 0.0)]).unwrap();
        }
        let person = book.get("ngoc").unwrap();
        assert!(person.centroids.len() <= MAX_CENTROIDS, "{}", person.centroids.len());
        assert_eq!(person.samples, 40, "every sample should still be counted");
    }

    #[test]
    fn correcting_the_model_teaches_it_rather_than_being_discarded() {
        // The user says a voice the model matched to Ngọc is actually Bình. That correction is the
        // most valuable sample there is, so it must be stored.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[vec![1.0, 0.0, 0.0, 0.0]]).unwrap();
        let disputed = vec![0.95, 0.31, 0.0, 0.0];
        assert!(matches!(book.identify(&disputed), Match::Known { .. }));

        book.enroll("Bình", std::slice::from_ref(&disputed)).unwrap();
        assert!(
            matches!(book.identify(&disputed), Match::Known { ref person, .. } if person == "binh"),
            "the correction did not win: {:?}",
            book.identify(&disputed)
        );
    }

    #[test]
    fn two_profiles_of_one_voice_can_be_merged() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[vec![1.0, 0.0, 0.0, 0.0]]).unwrap();
        book.enroll("Ngoc Nguyen", &[vec![0.0, 1.0, 0.0, 0.0]]).unwrap();

        book.merge("ngoc-nguyen", "ngoc").unwrap();
        assert_eq!(book.len(), 1);
        assert_eq!(book.get("ngoc").unwrap().samples, 2);
        assert!(matches!(book.identify(&[0.05, 1.0, 0.0, 0.0]), Match::Known { .. }));
    }

    #[test]
    fn merging_something_that_is_not_there_is_an_error_not_a_silent_no_op() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[voice(1.0, 0.0)]).unwrap();
        assert!(book.merge("nobody", "ngoc").is_err());
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut book = VoiceBook::default();
        assert!(book.enroll("   ", &[voice(1.0, 0.0)]).is_err());
    }

    #[test]
    fn the_book_survives_a_round_trip_to_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("voices.json");

        let mut book = VoiceBook::load(&path).unwrap();
        book.enroll("Ngọc", &[voice(1.0, 0.0)]).unwrap();
        book.set_avatar("ngoc", Some("attachments/ngoc.jpg".into())).unwrap();
        book.save().unwrap();

        let reloaded = VoiceBook::load(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.get("ngoc").unwrap().name, "Ngọc");
        assert_eq!(
            reloaded.get("ngoc").unwrap().avatar.as_deref(),
            Some("attachments/ngoc.jpg")
        );
        assert!(matches!(reloaded.identify(&voice(1.0, 0.02)), Match::Known { .. }));
    }

    #[test]
    fn a_first_run_has_met_nobody_and_that_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let book = VoiceBook::load(dir.path().join("nothing-here.json")).unwrap();
        assert!(book.is_empty());
        assert!(matches!(book.identify(&voice(1.0, 0.0)), Match::Unknown { .. }));
    }

    #[test]
    fn a_corrupt_book_is_reported_rather_than_silently_emptied() {
        // Losing every profile because a file got truncated should be loud, not a fresh start that
        // quietly stops recognising anyone.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("voices.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(VoiceBook::load(&path).is_err());
    }

    #[test]
    fn unnamed_voices_are_numbered_in_the_users_language() {
        assert_eq!(unknown_speaker(2).as_str(), "Người 2");
    }
}
