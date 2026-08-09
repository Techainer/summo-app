//! Deciding who is speaking, online.
//!
//! Frame-level diarization is expensive and, for a meeting recorder, mostly unnecessary. Summo has a
//! much cheaper signal available first: the microphone track is the local user by construction, so
//! only the remote track needs clustering at all.
//!
//! Within that track, this is deliberately not a diarization model. It embeds each *finished*
//! utterance — one vector for a few seconds of audio — and assigns it to a running set of speaker
//! centroids by cosine similarity. Embedding costs single-digit milliseconds per utterance and runs
//! after the segment is already on screen, so it never touches live latency.
//!
//! The interesting design choice is the uncertainty band. A single threshold forces every borderline
//! utterance into a decision it does not have the evidence for, and a wrong speaker label is far more
//! visible than a missing one — it puts words in someone's mouth. So similarities between the two
//! thresholds return [`Assignment::Uncertain`], the utterance is left unlabelled, and the offline
//! pass at the end of the meeting resolves it with the whole recording in hand.

use serde::{Deserialize, Serialize};
use summo_core::SpeakerId;

/// Tuning for [`OnlineClusterer`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// At or above this similarity, an utterance is confidently the same speaker.
    pub same_speaker: f32,
    /// Below this, it is confidently someone new. Between the two, we decline to guess.
    pub new_speaker: f32,
    /// Weight of a new sample when updating a centroid.
    ///
    /// Low, because a centroid should track a voice slowly: a single utterance recorded while
    /// someone leaned away from the microphone should not drag their profile with it.
    pub centroid_alpha: f32,
    /// Utterances shorter than this carry too little voice to embed reliably.
    pub min_duration_s: f64,
    /// Upper bound on distinct speakers before the least recently heard is evicted.
    pub max_speakers: usize,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            same_speaker: 0.62,
            new_speaker: 0.45,
            centroid_alpha: 0.15,
            min_duration_s: 0.8,
            max_speakers: 12,
        }
    }
}

/// What the clusterer concluded about one utterance.
#[derive(Debug, Clone, PartialEq)]
pub enum Assignment {
    /// Matched an existing speaker.
    Known { speaker: SpeakerId, similarity: f32 },
    /// Confidently a voice not heard before.
    New { speaker: SpeakerId },
    /// Similarity fell in the uncertainty band. Left unlabelled for the offline pass.
    Uncertain { best: SpeakerId, similarity: f32 },
    /// Too short to embed.
    TooShort,
}

impl Assignment {
    /// The label to show, if any. `Uncertain` deliberately yields nothing.
    #[must_use]
    pub fn speaker(&self) -> Option<&SpeakerId> {
        match self {
            Self::Known { speaker, .. } | Self::New { speaker } => Some(speaker),
            Self::Uncertain { .. } | Self::TooShort => None,
        }
    }
}

/// One tracked voice.
#[derive(Debug, Clone)]
struct Centroid {
    speaker: SpeakerId,
    vector: Vec<f32>,
    /// How many utterances have been folded in. A centroid with more evidence is more trustworthy.
    samples: u32,
    /// Monotonic counter for least-recently-used eviction.
    last_seen: u64,
}

/// Assigns utterance embeddings to speakers as a meeting progresses.
#[derive(Debug)]
pub struct OnlineClusterer {
    cfg: ClusterConfig,
    centroids: Vec<Centroid>,
    clock: u64,
    next_index: usize,
}

impl OnlineClusterer {
    #[must_use]
    pub fn new(cfg: ClusterConfig) -> Self {
        Self {
            cfg,
            centroids: Vec::new(),
            clock: 0,
            next_index: 0,
        }
    }

    #[must_use]
    pub fn speaker_count(&self) -> usize {
        self.centroids.len()
    }

    /// Every speaker discovered so far, in the order they first spoke.
    #[must_use]
    pub fn speakers(&self) -> Vec<SpeakerId> {
        self.centroids.iter().map(|c| c.speaker.clone()).collect()
    }

    /// Assign an utterance embedding to a speaker.
    ///
    /// `duration_s` gates the decision: a very short utterance is embedded from too little voice to
    /// be trusted, and guessing on it produces exactly the confident mislabelling this design is
    /// trying to avoid.
    pub fn assign(&mut self, embedding: &[f32], duration_s: f64) -> Assignment {
        if duration_s < self.cfg.min_duration_s || embedding.is_empty() {
            return Assignment::TooShort;
        }

        let query = normalize(embedding);
        self.clock += 1;

        let best = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, cosine(&query, &c.vector)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        match best {
            Some((index, similarity)) if similarity >= self.cfg.same_speaker => {
                let centroid = &mut self.centroids[index];
                blend(&mut centroid.vector, &query, self.cfg.centroid_alpha);
                centroid.samples += 1;
                centroid.last_seen = self.clock;
                Assignment::Known {
                    speaker: centroid.speaker.clone(),
                    similarity,
                }
            }
            Some((index, similarity)) if similarity >= self.cfg.new_speaker => {
                // The uncertainty band: plausible but not convincing. Say nothing rather than
                // attribute words to the wrong person.
                Assignment::Uncertain {
                    best: self.centroids[index].speaker.clone(),
                    similarity,
                }
            }
            _ => Assignment::New {
                speaker: self.add_speaker(query),
            },
        }
    }

    fn add_speaker(&mut self, vector: Vec<f32>) -> SpeakerId {
        if self.centroids.len() >= self.cfg.max_speakers {
            // A meeting with more than `max_speakers` voices is either a conference or a
            // misconfigured threshold; dropping the least recently heard keeps memory bounded and
            // favours whoever is still talking.
            if let Some(oldest) = self
                .centroids
                .iter()
                .enumerate()
                .min_by_key(|(_, c)| c.last_seen)
                .map(|(i, _)| i)
            {
                self.centroids.remove(oldest);
            }
        }

        let speaker = SpeakerId::auto(self.next_index);
        self.next_index += 1;
        self.centroids.push(Centroid {
            speaker: speaker.clone(),
            vector,
            samples: 1,
            last_seen: self.clock,
        });
        speaker
    }

    /// Re-cluster every utterance seen so far, with the whole meeting available.
    ///
    /// The online pass has to decide with only the past; this one can see that two utterances an
    /// hour apart are the same voice. Returns a mapping from utterance index to speaker, including
    /// the ones that were left uncertain live.
    #[must_use]
    pub fn refine(embeddings: &[(usize, Vec<f32>)], cfg: ClusterConfig) -> Vec<(usize, SpeakerId)> {
        if embeddings.is_empty() {
            return Vec::new();
        }

        let normalized: Vec<(usize, Vec<f32>)> =
            embeddings.iter().map(|(i, v)| (*i, normalize(v))).collect();

        // Agglomerative clustering with a single linkage threshold. Every utterance joins the
        // cluster whose centroid it is closest to, if that similarity clears the bar; otherwise it
        // starts one. Two passes, so an early utterance benefits from a centroid built later.
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        let mut centroids: Vec<Vec<f32>> = Vec::new();

        for _ in 0..2 {
            for (position, (_, vector)) in normalized.iter().enumerate() {
                let best = centroids
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, cosine(vector, c)))
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

                match best {
                    Some((index, similarity)) if similarity >= cfg.same_speaker => {
                        if !clusters[index].contains(&position) {
                            clusters[index].push(position);
                            blend(&mut centroids[index], vector, cfg.centroid_alpha);
                        }
                    }
                    _ => {
                        if clusters.iter().all(|c| !c.contains(&position)) {
                            clusters.push(vec![position]);
                            centroids.push(vector.clone());
                        }
                    }
                }
            }
        }

        // Number speakers by when they first spoke, so labels read in meeting order.
        let mut order: Vec<(usize, usize)> = clusters
            .iter()
            .enumerate()
            .filter_map(|(cluster, members)| members.iter().min().map(|first| (cluster, *first)))
            .collect();
        order.sort_by_key(|(_, first)| *first);

        let mut out = Vec::new();
        for (label, (cluster, _)) in order.into_iter().enumerate() {
            let speaker = SpeakerId::auto(label);
            for &position in &clusters[cluster] {
                out.push((normalized[position].0, speaker.clone()));
            }
        }
        out.sort_by_key(|(index, _)| *index);
        out
    }
}

/// Scale a vector to unit length so cosine similarity is a dot product.
fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Cosine similarity of two already-normalised vectors.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| x * y)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

/// Move `target` a fraction `alpha` toward `sample`, keeping it unit length.
fn blend(target: &mut Vec<f32>, sample: &[f32], alpha: f32) {
    if target.len() != sample.len() {
        return;
    }
    for (t, s) in target.iter_mut().zip(sample) {
        *t = *t * (1.0 - alpha) + s * alpha;
    }
    *target = normalize(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic embedding: a unit vector pointing mostly along axis `id`, with `noise` mixed in
    /// from a neighbouring axis. Two utterances from one speaker differ only in noise.
    fn voice(id: usize, noise: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 16];
        v[id % 16] = 1.0;
        v[(id + 1) % 16] = noise;
        normalize(&v)
    }

    #[test]
    fn the_first_voice_starts_a_speaker() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        let a = c.assign(&voice(0, 0.05), 2.0);
        assert!(matches!(a, Assignment::New { .. }));
        assert_eq!(a.speaker().unwrap().as_str(), "S1");
        assert_eq!(c.speaker_count(), 1);
    }

    #[test]
    fn the_same_voice_returns_the_same_label() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        c.assign(&voice(0, 0.05), 2.0);
        let again = c.assign(&voice(0, 0.08), 2.0);

        assert!(matches!(again, Assignment::Known { .. }), "got {again:?}");
        assert_eq!(again.speaker().unwrap().as_str(), "S1");
        assert_eq!(c.speaker_count(), 1, "one voice must not become two");
    }

    #[test]
    fn a_different_voice_gets_its_own_label() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        c.assign(&voice(0, 0.05), 2.0);
        let other = c.assign(&voice(8, 0.05), 2.0);

        assert!(matches!(other, Assignment::New { .. }));
        assert_eq!(other.speaker().unwrap().as_str(), "S2");
        assert_eq!(c.speaker_count(), 2);
    }

    #[test]
    fn a_borderline_match_is_left_unlabelled_rather_than_guessed() {
        // Wide band so a deliberately ambiguous vector lands inside it.
        let cfg = ClusterConfig {
            same_speaker: 0.95,
            new_speaker: 0.30,
            ..ClusterConfig::default()
        };
        let mut c = OnlineClusterer::new(cfg);
        c.assign(&voice(0, 0.0), 2.0);

        let ambiguous = c.assign(&voice(0, 0.9), 2.0);

        assert!(
            matches!(ambiguous, Assignment::Uncertain { .. }),
            "got {ambiguous:?}"
        );
        assert!(
            ambiguous.speaker().is_none(),
            "an uncertain assignment must not put words in someone's mouth"
        );
        assert_eq!(
            c.speaker_count(),
            1,
            "uncertainty must not invent a speaker either"
        );
    }

    #[test]
    fn short_utterances_are_not_attributed() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        let short = c.assign(&voice(0, 0.05), 0.3);
        assert_eq!(short, Assignment::TooShort);
        assert_eq!(
            c.speaker_count(),
            0,
            "a 300 ms grunt should not create a speaker"
        );
    }

    #[test]
    fn an_empty_embedding_is_refused() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        assert_eq!(c.assign(&[], 5.0), Assignment::TooShort);
    }

    #[test]
    fn a_conversation_alternating_between_two_people_stays_at_two() {
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        let mut labels = Vec::new();
        for turn in 0..10 {
            let id = if turn % 2 == 0 { 0 } else { 8 };
            let noise = 0.05 + (turn as f32) * 0.005;
            let a = c.assign(&voice(id, noise), 2.5);
            labels.push(a.speaker().map(|s| s.as_str().to_string()));
        }

        assert_eq!(c.speaker_count(), 2, "labels drifted: {labels:?}");
        assert_eq!(labels[0], labels[2], "the same voice must keep its label");
        assert_eq!(labels[1], labels[3]);
        assert_ne!(labels[0], labels[1]);
    }

    #[test]
    fn the_speaker_bank_is_bounded() {
        let cfg = ClusterConfig {
            max_speakers: 3,
            ..ClusterConfig::default()
        };
        let mut c = OnlineClusterer::new(cfg);
        for id in 0..8 {
            c.assign(&voice(id * 2, 0.01), 2.0);
        }
        assert_eq!(
            c.speaker_count(),
            3,
            "memory must stay bounded on a large call"
        );
    }

    #[test]
    fn centroids_move_slowly_toward_new_evidence() {
        // A voice recorded further from the microphone should nudge the profile, not replace it.
        let mut c = OnlineClusterer::new(ClusterConfig::default());
        c.assign(&voice(0, 0.0), 2.0);
        let before = c.centroids[0].vector.clone();

        c.assign(&voice(0, 0.3), 2.0);
        let after = &c.centroids[0].vector;

        let moved = cosine(&before, after);
        assert!(moved > 0.9, "centroid jumped too far: similarity {moved}");
        assert!(moved < 1.0, "centroid should have moved at least a little");
    }

    #[test]
    fn offline_refinement_labels_by_first_appearance() {
        // Three utterances: B speaks first in the list, then A, then B again.
        let embeddings = vec![
            (0, voice(8, 0.02)),
            (1, voice(0, 0.02)),
            (2, voice(8, 0.04)),
        ];
        let out = OnlineClusterer::refine(&embeddings, ClusterConfig::default());

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].1.as_str(), "S1", "the first voice heard is S1");
        assert_eq!(out[1].1.as_str(), "S2");
        assert_eq!(out[2].1, out[0].1, "the same voice must get the same label");
    }

    #[test]
    fn refinement_resolves_what_the_live_pass_left_uncertain() {
        // Live, an utterance seen before its speaker had a stable centroid may be unlabelled.
        // Offline, with every utterance available, it should be attributed.
        let embeddings = vec![
            (0, voice(0, 0.10)),
            (1, voice(0, 0.02)),
            (2, voice(0, 0.06)),
            (3, voice(8, 0.02)),
        ];
        let out = OnlineClusterer::refine(&embeddings, ClusterConfig::default());

        assert_eq!(out.len(), 4, "every utterance should end up labelled");
        assert_eq!(out[0].1, out[1].1);
        assert_eq!(out[1].1, out[2].1);
        assert_ne!(out[3].1, out[0].1);
    }

    #[test]
    fn refining_nothing_is_not_a_panic() {
        assert!(OnlineClusterer::refine(&[], ClusterConfig::default()).is_empty());
    }

    #[test]
    fn cosine_of_mismatched_lengths_is_zero_not_a_panic() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn normalizing_a_zero_vector_does_not_divide_by_zero() {
        let z = normalize(&[0.0, 0.0, 0.0]);
        assert!(z.iter().all(|x| *x == 0.0));
    }
}
