//! The voice book: who Summo has learned to recognise, kept between meetings in `~/.summo/voices`.
//!
//! [`crate::cluster`] separates voices *within* one meeting and calls them `S1`, `S2`. This is the
//! layer above: it remembers that the voice called `S2` last Tuesday belongs to Ngọc, so the next
//! meeting says "Ngọc" without being asked again.
//!
//! Three decisions shape everything here.
//!
//! **A profile is samples, and centroids are derived from them.** The obvious design keeps a
//! running average per person and folds each new utterance into it. That design cannot be
//! corrected: once a stranger's voice has been averaged into Ngọc there is no subtraction that
//! takes it back out, and every future meeting inherits the mistake. Keeping the samples means a
//! wrong one can simply be dropped and the centroids recomputed.
//!
//! **One person is several centroids, not one.** The same voice over a headset, a phone and a
//! laptop microphone lands in noticeably different places; averaging them produces a centroid that
//! matches none of them. Centroids are derived by merging only samples that are genuinely alike, so
//! the spread of a voice survives instead of being flattened.
//!
//! **A human's answer outranks the model, and is allowed to take things away.** When a user says a
//! line attributed to Ngọc was actually Bình, the samples that caused that match leave Ngọc's
//! profile — otherwise the next meeting makes the same mistake, the user corrects it again, and the
//! product looks like it does not learn.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, SpeakerId};

/// Samples kept per person.
///
/// Bounded because a profile is a description of a voice, not a recording of everything it ever
/// said. Thirty-two vectors is about 24 KB and covers every device and mood one person has.
const MAX_SAMPLES: usize = 32;

/// Centroids derived per person.
const MAX_CENTROIDS: usize = 8;

/// Samples this similar describe the same recording conditions and are merged into one centroid.
///
/// Deliberately high: merging too eagerly is what collapses a voice's spread into a centroid that
/// matches none of its variants.
const MERGE_SIMILARITY: f32 = 0.80;

/// How confidently a stored voice has to match before a name is used.
///
/// Higher than the within-meeting threshold on purpose: calling somebody by the wrong *name* across
/// meetings is a worse failure than leaving two utterances in one meeting unmerged.
pub const MATCH_THRESHOLD: f32 = 0.68;

/// Below this, the voice is confidently nobody known.
pub const REJECT_THRESHOLD: f32 = 0.52;

/// Samples at least this similar to a reassigned one are the same voice, and leave the profile they
/// were wrongly filed under.
pub const SAME_VOICE: f32 = 0.62;

/// One utterance that went into a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub embedding: Vec<f32>,
    /// True when a human asserted this. Confirmed samples are never evicted, and never taken away
    /// by a later correction.
    #[serde(default)]
    pub confirmed: bool,
    /// Insertion order, for evicting the oldest guess when the profile is full.
    #[serde(default)]
    pub age: u64,
}

/// A person Summo can recognise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// Stable key, used in file names and wikilinks.
    pub id: String,
    pub name: String,
    /// What this voice has actually sounded like.
    #[serde(default)]
    pub samples: Vec<Sample>,
    /// Derived from `samples`; stored so a load does not have to recompute them.
    #[serde(default)]
    pub centroids: Vec<Vec<f32>>,
    /// Path relative to the vault, e.g. `attachments/ngoc.jpg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default)]
    next_age: u64,
}

impl Person {
    pub(crate) fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            samples: Vec::new(),
            centroids: Vec::new(),
            avatar: None,
            next_age: 0,
        }
    }

    /// How many utterances shaped this profile.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// How many were asserted by a person rather than guessed.
    #[must_use]
    pub fn confirmed_count(&self) -> usize {
        self.samples.iter().filter(|s| s.confirmed).count()
    }

    /// Best similarity between `embedding` and any centroid.
    #[must_use]
    pub fn similarity(&self, embedding: &[f32]) -> f32 {
        self.centroids
            .iter()
            .map(|c| cosine(c, embedding))
            .fold(0.0, f32::max)
    }

    /// Add an utterance to the profile.
    pub fn absorb(&mut self, embedding: &[f32], confirmed: bool) {
        if embedding.is_empty() {
            return;
        }
        self.samples.push(Sample {
            embedding: normalize(embedding),
            confirmed,
            age: self.next_age,
        });
        self.next_age += 1;
        self.evict();
        self.rebuild();
    }

    /// Drop samples that sound like any of `embeddings`, and say how many went.
    ///
    /// This is the half of correction the obvious design cannot do. Confirmed samples stay: a human
    /// already asserted those, and when two human answers conflict the book keeps both rather than
    /// letting the newer one silently erase the older.
    pub fn remove_like(&mut self, embeddings: &[Vec<f32>]) -> usize {
        let before = self.samples.len();
        self.samples.retain(|sample| {
            if sample.confirmed {
                return true;
            }
            !embeddings
                .iter()
                .any(|e| cosine(&sample.embedding, e) >= SAME_VOICE)
        });
        let removed = before - self.samples.len();
        if removed > 0 {
            self.rebuild();
        }
        removed
    }

    /// Keep the profile bounded, discarding guesses before anything a human confirmed.
    fn evict(&mut self) {
        while self.samples.len() > MAX_SAMPLES {
            let victim = self
                .samples
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.confirmed)
                .min_by_key(|(_, s)| s.age)
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.samples.remove(i);
                }
                // Every sample is confirmed. Dropping a human's answer to make room for another
                // would be the wrong trade, so the profile is allowed over budget instead.
                None => break,
            }
        }
    }

    /// Recompute centroids from the samples, merging only the ones that are genuinely alike.
    ///
    /// Agglomerative rather than k-means: how many ways a voice sounds is not known in advance, and
    /// asking for a fixed number of clusters either invents distinctions or erases them.
    fn rebuild(&mut self) {
        let mut clusters: Vec<(Vec<f32>, usize)> = self
            .samples
            .iter()
            .map(|s| (s.embedding.clone(), 1))
            .collect();

        while clusters.len() > 1 {
            let mut best = (0usize, 1usize, f32::MIN);
            for i in 0..clusters.len() {
                for j in (i + 1)..clusters.len() {
                    let similarity = cosine(&clusters[i].0, &clusters[j].0);
                    if similarity > best.2 {
                        best = (i, j, similarity);
                    }
                }
            }
            let (i, j, similarity) = best;
            // Stop once what remains are genuinely different ways this voice sounds — unless there
            // are still more of them than the cap allows.
            if similarity < MERGE_SIMILARITY && clusters.len() <= MAX_CENTROIDS {
                break;
            }
            let (other, weight) = clusters.remove(j);
            let (centroid, count) = &mut clusters[i];
            let total = (*count + weight) as f32;
            for (slot, value) in centroid.iter_mut().zip(&other) {
                *slot = (*slot * *count as f32 + value * weight as f32) / total;
            }
            let renormalized = normalize(centroid);
            centroid.copy_from_slice(&renormalized);
            *count += weight;
        }

        self.centroids = clusters.into_iter().map(|(c, _)| c).collect();
    }
}

/// What the book concluded about a voice.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Confidently a person already known.
    Known { person: String, similarity: f32 },
    /// Similar to somebody, but not enough to put their name on it.
    ///
    /// Deliberately not a label: showing the wrong name is worse than showing none, because a
    /// reader trusts a name and does not go back to the audio to check it.
    Unsure { person: String, similarity: f32 },
    /// Nobody known sounds like this.
    Unknown { best: f32 },
}

impl Match {
    /// The person id to use, if any.
    #[must_use]
    pub fn person(&self) -> Option<&str> {
        match self {
            Self::Known { person, .. } => Some(person),
            Self::Unsure { .. } | Self::Unknown { .. } => None,
        }
    }
}

/// What a correction did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reassignment {
    /// The person the utterances now belong to.
    pub person: String,
    /// Profiles the samples were taken out of, and how many left each.
    pub removed_from: Vec<(String, usize)>,
    /// Profiles left with nothing, which were deleted.
    pub emptied: Vec<String>,
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

    /// The display name for a person id, falling back to the id itself.
    #[must_use]
    pub fn name_of(&self, id: &str) -> String {
        self.people
            .get(id)
            .map_or_else(|| id.to_string(), |p| p.name.clone())
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
    /// Additive: it takes nothing away from anybody else. Use [`Self::reassign`] when the
    /// utterances are currently attributed to the wrong person — that case needs a removal as well
    /// as an addition.
    pub fn enroll(
        &mut self,
        name: &str,
        embeddings: &[Vec<f32>],
        confirmed: bool,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Other("a person needs a name".into()));
        }
        let id = slug(name);

        let person = self
            .people
            .entry(id.clone())
            .or_insert_with(|| Person::new(id.clone(), name.to_string()));
        // A rename keeps the same id, so links and history survive being corrected.
        person.name = name.to_string();
        for embedding in embeddings {
            person.absorb(embedding, confirmed);
        }
        Ok(id)
    }

    /// A human says these utterances are `name`, not whoever they were attributed to.
    ///
    /// This is the correction that keeps the book honest. Adding the samples to the right person is
    /// only half of it: the samples that produced the wrong match are taken out of the profile they
    /// polluted, or the next meeting makes the same mistake and the product looks like it does not
    /// learn.
    ///
    /// A profile left with nothing is deleted. It described a voice that turned out to be somebody
    /// else's; keeping it would match nothing while cluttering the list of people.
    pub fn reassign(&mut self, name: &str, embeddings: &[Vec<f32>]) -> Result<Reassignment> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Other("a person needs a name".into()));
        }
        let target = slug(name);

        let mut removed_from = Vec::new();
        for person in self.people.values_mut() {
            if person.id == target {
                continue;
            }
            let removed = person.remove_like(embeddings);
            if removed > 0 {
                removed_from.push((person.id.clone(), removed));
            }
        }

        let emptied: Vec<String> = self
            .people
            .values()
            .filter(|p| p.samples.is_empty() && p.id != target)
            .map(|p| p.id.clone())
            .collect();
        for id in &emptied {
            self.people.remove(id);
        }

        let person = self.enroll(name, embeddings, true)?;
        Ok(Reassignment {
            person,
            removed_from,
            emptied,
        })
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
        for sample in source.samples {
            target.absorb(&sample.embedding, sample.confirmed);
        }
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

pub(crate) fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
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

    /// Voices far apart in space, so a match is unambiguous.
    const NGOC: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const BINH: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const KHACH: [f32; 4] = [0.0, 0.0, 1.0, 0.0];

    fn near(v: [f32; 4], jitter: f32) -> Vec<f32> {
        normalize(&[v[0] + jitter, v[1] + jitter, v[2], v[3]])
    }

    #[test]
    fn a_named_voice_is_recognised_next_time() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        assert_eq!(book.identify(&near(NGOC, 0.02)).person(), Some("ngoc"));
    }

    #[test]
    fn a_voice_nobody_knows_is_not_given_someone_elses_name() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        assert!(matches!(book.identify(&KHACH), Match::Unknown { .. }));
    }

    #[test]
    fn a_borderline_voice_is_left_unnamed_rather_than_guessed() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        // Cosine 0.6 — between the reject and match thresholds.
        let m = book.identify(&[0.6, 0.8, 0.0, 0.0]);
        assert!(matches!(m, Match::Unsure { .. }), "got {m:?}");
        assert_eq!(m.person(), None);
    }

    #[test]
    fn names_that_differ_only_by_tone_marks_are_the_same_person() {
        let mut book = VoiceBook::default();
        let a = book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        let b = book.enroll("ngoc", &[near(NGOC, 0.01)], true).unwrap();
        assert_eq!(a, b);
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn one_person_keeps_a_centroid_per_way_they_sound() {
        // A headset and a phone put the same voice in different places. Collapsing them into one
        // centroid leaves a profile that matches neither.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec(), BINH.to_vec()], true)
            .unwrap();

        assert_eq!(
            book.get("ngoc").unwrap().centroids.len(),
            2,
            "the spread of the voice was flattened"
        );
        assert!(matches!(book.identify(&near(NGOC, 0.02)), Match::Known { .. }));
        assert!(matches!(book.identify(&near(BINH, 0.02)), Match::Known { .. }));
    }

    #[test]
    fn near_identical_recordings_do_not_each_earn_a_centroid() {
        let mut book = VoiceBook::default();
        let samples: Vec<Vec<f32>> = (0..12).map(|i| near(NGOC, i as f32 * 0.002)).collect();
        book.enroll("Ngọc", &samples, true).unwrap();

        let person = book.get("ngoc").unwrap();
        assert_eq!(person.centroids.len(), 1);
        assert_eq!(person.sample_count(), 12, "the samples themselves are kept");
    }

    #[test]
    fn a_profile_stays_bounded_and_keeps_what_a_human_confirmed() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        let guesses: Vec<Vec<f32>> = (0..80).map(|i| near(NGOC, i as f32 * 0.001)).collect();
        book.enroll("Ngọc", &guesses, false).unwrap();

        let person = book.get("ngoc").unwrap();
        assert!(
            person.sample_count() <= MAX_SAMPLES,
            "{} samples",
            person.sample_count()
        );
        assert_eq!(person.confirmed_count(), 1, "a human's answer was evicted");
        assert!(person.centroids.len() <= MAX_CENTROIDS);
    }

    #[test]
    fn a_wrong_match_is_taken_back_out_of_the_profile_it_polluted() {
        // The case that matters. A third person is recognised as Ngọc and the user fixes it. Adding
        // the samples to Bình is not enough: while they remain in Ngọc's profile the next meeting
        // makes the same mistake, the user corrects it again, and the product looks like it does
        // not learn.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();

        let stranger: Vec<Vec<f32>> = (0..3).map(|i| near(KHACH, i as f32 * 0.01)).collect();
        book.enroll("Ngọc", &stranger, false).unwrap();
        assert_eq!(book.identify(&KHACH).person(), Some("ngoc"), "setup failed");

        let done = book.reassign("Bình", &stranger).unwrap();
        assert_eq!(done.person, "binh");
        assert_eq!(done.removed_from, vec![("ngoc".to_string(), 3)]);

        assert_eq!(book.identify(&KHACH).person(), Some("binh"));
        assert_eq!(book.identify(&near(NGOC, 0.01)).person(), Some("ngoc"));
        assert_eq!(book.get("ngoc").unwrap().sample_count(), 1);
    }

    #[test]
    fn reassigning_does_not_strip_what_a_human_already_confirmed() {
        // Two human answers conflict. Keeping both is honest; letting the newer one silently erase
        // the older would lose an assertion nobody withdrew.
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        book.reassign("Bình", &[near(NGOC, 0.005)]).unwrap();

        assert_eq!(book.get("ngoc").unwrap().confirmed_count(), 1);
        assert_eq!(book.get("binh").unwrap().confirmed_count(), 1);
    }

    #[test]
    fn a_profile_that_was_entirely_somebody_else_is_removed() {
        let mut book = VoiceBook::default();
        let stranger: Vec<Vec<f32>> = (0..2).map(|i| near(KHACH, i as f32 * 0.01)).collect();
        book.enroll("Người 2", &stranger, false).unwrap();

        let done = book.reassign("Bình", &stranger).unwrap();
        assert_eq!(done.emptied, vec!["nguoi-2".to_string()]);
        assert!(book.get("nguoi-2").is_none());
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn reassigning_to_whoever_already_has_them_only_confirms() {
        let mut book = VoiceBook::default();
        let voice = vec![near(NGOC, 0.0)];
        book.enroll("Ngọc", &voice, false).unwrap();

        let done = book.reassign("Ngọc", &voice).unwrap();
        assert!(done.removed_from.is_empty(), "it removed from itself");
        assert_eq!(book.len(), 1);
        assert_eq!(book.get("ngoc").unwrap().confirmed_count(), 1);
    }

    #[test]
    fn two_profiles_of_one_voice_can_be_merged() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        book.enroll("Ngoc Nguyen", &[BINH.to_vec()], true).unwrap();

        book.merge("ngoc-nguyen", "ngoc").unwrap();
        assert_eq!(book.len(), 1);
        assert_eq!(book.get("ngoc").unwrap().sample_count(), 2);
        assert!(matches!(book.identify(&near(BINH, 0.02)), Match::Known { .. }));
    }

    #[test]
    fn merging_something_that_is_not_there_is_an_error_not_a_silent_no_op() {
        let mut book = VoiceBook::default();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        assert!(book.merge("nobody", "ngoc").is_err());
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut book = VoiceBook::default();
        assert!(book.enroll("   ", &[NGOC.to_vec()], true).is_err());
        assert!(book.reassign("  ", &[NGOC.to_vec()]).is_err());
    }

    #[test]
    fn the_book_survives_a_round_trip_and_is_still_correctable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("voices.json");

        let mut book = VoiceBook::load(&path).unwrap();
        book.enroll("Ngọc", &[NGOC.to_vec()], true).unwrap();
        book.enroll("Ngọc", &[BINH.to_vec()], false).unwrap();
        book.set_avatar("ngoc", Some("attachments/ngoc.jpg".into()))
            .unwrap();
        book.save().unwrap();

        let mut reloaded = VoiceBook::load(&path).unwrap();
        assert_eq!(reloaded.get("ngoc").unwrap().name, "Ngọc");
        assert_eq!(reloaded.get("ngoc").unwrap().centroids.len(), 2);
        assert_eq!(
            reloaded.get("ngoc").unwrap().avatar.as_deref(),
            Some("attachments/ngoc.jpg")
        );

        // Correcting after a reload needs the samples themselves to have survived, not just the
        // centroids derived from them.
        reloaded.reassign("Bình", &[BINH.to_vec()]).unwrap();
        assert_eq!(reloaded.get("ngoc").unwrap().sample_count(), 1);
    }

    #[test]
    fn a_first_run_has_met_nobody_and_that_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let book = VoiceBook::load(dir.path().join("nothing-here.json")).unwrap();
        assert!(book.is_empty());
        assert!(matches!(book.identify(&NGOC), Match::Unknown { .. }));
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
