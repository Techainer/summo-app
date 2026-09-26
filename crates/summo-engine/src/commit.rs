//! Deciding which words are settled enough to act on, before the sentence has finished.
//!
//! The recogniser re-decodes the open utterance about every 150 ms and emits the result as partial
//! text, then emits a final when the gate closes — which needs 400 ms of trailing silence first.
//! Everything downstream waits for that final: translation, and with it any subtitle or spoken dub.
//! So the cheapest second of the whole pipeline is the one spent waiting for somebody to stop
//! talking, and it is spent on every sentence.
//!
//! Partial text is already on screen. What is missing is a rule for when a *piece* of it is safe to
//! translate, and this is that rule.
//!
//! ## Local agreement
//!
//! A partial is a guess that gets revised. Two consecutive guesses agreeing about a prefix is weak
//! evidence that the prefix is right — weak, but much stronger than one guess — and it costs
//! nothing to compute. So: keep the longest prefix the last two partials agree on, and hand out
//! whatever new *clauses* it contains.
//!
//! Clauses, not words. A clause boundary is a place the model has already decided a phrase ends, so
//! cutting there never splits a word and rarely splits a thought — and a translator handed half a
//! phrase produces half a translation, confidently.
//!
//! ## What this guarantees, and what it does not
//!
//! It guarantees **this module never takes a word back**. Each piece it hands out continues the
//! last one; nothing is ever re-issued, corrected or withdrawn.
//!
//! It does **not** guarantee the words were right. Local agreement makes revision unlikely, not
//! impossible, and a model that changes its mind about text already handed out cannot be un-said.
//! That is the whole reason the two consumers differ:
//!
//! * A **subtitle** may take pieces as they come. If the final disagrees it overwrites, which is
//!   what [`crate::protocol`]'s `accepts` already allows for.
//! * A **spoken dub** may too, and the cost of being wrong is higher, because speech cannot be
//!   redrawn. [`Committer::revisions`] counts how often it happened so the trade is a number
//!   somebody can look at rather than a hope.
//!
//! No clock and no model in here, which is what makes every rule testable without sleeping.

/// Where a clause can end.
///
/// Latin and CJK, because a meeting has both in it and a rule that knows only `.` and `,` would
/// commit nothing at all in Japanese or Chinese — the languages where waiting for the end of the
/// sentence costs the most, since their word order defers the verb.
const BOUNDARIES: [char; 11] = ['.', ',', ';', '?', '!', '…', '。', '、', '；', '？', '！'];

/// Shortest piece worth handing out on its own, in the units of [`weight`].
///
/// A two-character clause saves nothing: the point of committing early is to start translating —
/// and speaking — before the sentence ends, and at roughly fifteen Latin characters a second of
/// speech this is about eight tenths of a second of it. Below that the piece costs a separate
/// request and a separate synthesis to buy less time than either takes.
///
/// A judgement, not a measurement. It is a named constant so it can be argued with.
pub const MIN_WEIGHT: usize = 12;

/// Roughly how much speech a string is, in Latin characters.
///
/// Counting characters directly is wrong across scripts and wrong in the direction that matters.
/// `今日は予算について、` is ten characters and a complete clause — two and a half seconds of
/// speech — while ten Latin characters is one word. A flat threshold either commits nothing in
/// Japanese and Chinese, which are the languages where waiting for the end of the sentence costs
/// the most because the verb is last, or commits fragments in Vietnamese and English.
///
/// So an ideograph, a kana or a Hangul syllable counts as three. That is the ratio between the
/// two scripts' characters-per-second, near enough for a floor whose whole job is to reject
/// fragments; it is not trying to predict a duration.
#[must_use]
pub fn weight(text: &str) -> usize {
    text.chars()
        .map(|c| match c {
            '\u{4E00}'..='\u{9FFF}'   // CJK ideographs
            | '\u{3040}'..='\u{30FF}' // hiragana and katakana
            | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
                => 3,
            _ => 1,
        })
        .sum()
}

/// One settled piece of an utterance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// The utterance it belongs to, so a consumer can attach it to the right line.
    pub seq: u64,
    /// The new text, continuing whatever was handed out before it for this `seq`.
    pub text: String,
    /// Whether the utterance is now complete. The last piece of every utterance has this set, and
    /// a consumer that buffers per sentence knows it can stop waiting.
    pub last: bool,
}

/// Tracks one utterance at a time and decides what is settled.
///
/// One at a time because the recogniser only has one utterance open at a time per lane: a final
/// closes it before the next partial opens the next. A new `seq` therefore means the previous
/// utterance is over, which is handled rather than assumed — a dropped final must not wedge this
/// on a sentence that ended a minute ago.
#[derive(Debug, Default)]
pub struct Committer {
    /// The utterance being tracked, if any.
    seq: Option<u64>,
    /// The previous partial, for the agreement comparison.
    previous: String,
    /// Everything handed out for this utterance so far, joined.
    given: String,
    /// Times a final disagreed with text already handed out.
    revisions: usize,
}

impl Committer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer a partial. Returns a piece when one has settled.
    pub fn partial(&mut self, seq: u64, text: &str) -> Option<Piece> {
        self.reseat(seq);

        let agreed = common_prefix(&self.previous, text);
        self.previous = text.to_string();

        // Only up to the last boundary inside the agreed prefix. Past it the words are agreed but
        // the phrase is not finished, and half a phrase translates into half a thought.
        let upto = last_boundary(&text[..agreed])?;
        self.hand_out(seq, &text[..upto], false)
    }

    /// Offer the final text. Returns whatever is left, and closes the utterance.
    ///
    /// Always returns the tail even when nothing was committed early, so an utterance with no
    /// punctuation in it — which is most short ones — still arrives whole.
    pub fn settle(&mut self, seq: u64, text: &str) -> Option<Piece> {
        self.reseat(seq);

        // The final is the authority. If it disagrees with what was already handed out, the words
        // spoken from those pieces were wrong and cannot be recalled; what *can* be avoided is
        // saying the disputed part twice, so the tail is measured from where the two stop agreeing.
        if !text.starts_with(&self.given) && !self.given.is_empty() {
            self.revisions += 1;
        }

        let out = self.hand_out(seq, text, true);
        self.seq = None;
        self.previous.clear();
        self.given.clear();
        out
    }

    /// How often a final contradicted text already handed out.
    ///
    /// The cost of committing early, counted rather than assumed. A consumer that cannot take words
    /// back — a spoken dub — is the one that should be reading this.
    #[must_use]
    pub fn revisions(&self) -> usize {
        self.revisions
    }

    /// Forget the open utterance, for the end of a session.
    pub fn reset(&mut self) {
        self.seq = None;
        self.previous.clear();
        self.given.clear();
    }

    /// Start tracking `seq` if it is not the one being tracked.
    ///
    /// A new utterance arriving without its predecessor's final is not an error to report: a final
    /// can be filtered out as a hallucination, and a committer wedged on a sentence nobody will
    /// ever finish would silently stop the feature for the rest of the meeting.
    fn reseat(&mut self, seq: u64) {
        if self.seq != Some(seq) {
            self.seq = Some(seq);
            self.previous.clear();
            self.given.clear();
        }
    }

    /// Hand out whatever of `settled` has not been handed out yet.
    fn hand_out(&mut self, seq: u64, settled: &str, last: bool) -> Option<Piece> {
        let shared = common_prefix(&self.given, settled);
        let fresh = settled[shared..].trim();

        // A final always closes the utterance, even when it adds nothing — a consumer buffering per
        // sentence has to be told the sentence is over. A partial that adds too little waits: it
        // will be part of a longer piece a moment later, which is strictly better than two requests.
        if fresh.is_empty() && !last {
            return None;
        }
        if !last && weight(fresh) < MIN_WEIGHT {
            return None;
        }
        if fresh.is_empty() && last {
            // Nothing new to say, but the utterance is over. Reported with empty text rather than
            // not at all: `None` here would mean "still open" to anybody waiting for `last`.
            return Some(Piece {
                seq,
                text: String::new(),
                last: true,
            });
        }

        self.given = settled.to_string();
        Some(Piece {
            seq,
            text: fresh.to_string(),
            last,
        })
    }
}

/// Byte length of the longest prefix two strings share, cut to a character boundary.
///
/// Byte-wise with a boundary check rather than zipping characters: the strings are usually
/// identical for most of their length, and slicing a `str` at a byte that is not a boundary panics.
#[must_use]
fn common_prefix(a: &str, b: &str) -> usize {
    let limit = a.len().min(b.len());
    let mut shared = 0;
    for i in 0..limit {
        if a.as_bytes()[i] != b.as_bytes()[i] {
            break;
        }
        // Only advance the answer to somewhere both strings can actually be cut.
        if a.is_char_boundary(i + 1) {
            shared = i + 1;
        }
    }
    shared
}

/// Byte offset just past the last clause boundary in `text`, if there is one.
#[must_use]
fn last_boundary(text: &str) -> Option<usize> {
    text.char_indices()
        .rfind(|(_, c)| BOUNDARIES.contains(c))
        .map(|(i, c)| i + c.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a sequence of partials and a final, collecting everything handed out.
    fn run(seq: u64, partials: &[&str], final_text: &str) -> Vec<Piece> {
        let mut committer = Committer::new();
        let mut out: Vec<Piece> = partials
            .iter()
            .filter_map(|text| committer.partial(seq, text))
            .collect();
        out.extend(committer.settle(seq, final_text));
        out
    }

    /// Rejoining the pieces has to give back the final text. Anything else means a word was lost or
    /// said twice, and both are worse than the wait this replaces.
    fn joined(pieces: &[Piece]) -> String {
        pieces
            .iter()
            .map(|p| p.text.as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The whole point: a clause is handed out before the speaker has stopped.
    #[test]
    fn a_settled_clause_goes_out_before_the_sentence_ends() {
        let pieces = run(
            1,
            &[
                "Chúng ta cần",
                "Chúng ta cần bàn về ngân sách,",
                "Chúng ta cần bàn về ngân sách, và tôi nghĩ",
            ],
            "Chúng ta cần bàn về ngân sách, và tôi nghĩ con số sai.",
        );

        assert!(pieces.len() >= 2, "nothing was committed early: {pieces:?}");
        assert_eq!(pieces[0].text, "Chúng ta cần bàn về ngân sách,");
        assert!(!pieces[0].last);
        assert!(pieces.last().unwrap().last);
        assert_eq!(
            joined(&pieces),
            "Chúng ta cần bàn về ngân sách, và tôi nghĩ con số sai."
        );
    }

    /// One partial is a guess. A clause that has appeared exactly once is not settled, however
    /// confident the punctuation looks.
    #[test]
    fn one_partial_agrees_with_nothing_and_commits_nothing() {
        let mut committer = Committer::new();
        assert_eq!(committer.partial(1, "Chúng ta cần bàn về ngân sách,"), None);
    }

    /// The failure this module must not have. Every piece continues the last one.
    #[test]
    fn nothing_handed_out_is_ever_taken_back() {
        let pieces = run(
            1,
            &[
                "Hôm nay",
                "Hôm nay chúng ta họp,",
                "Hôm nay chúng ta họp,",
                "Hôm nay chúng ta họp, về ngân sách,",
                "Hôm nay chúng ta họp, về ngân sách, rồi",
            ],
            "Hôm nay chúng ta họp, về ngân sách, rồi nghỉ.",
        );

        let mut so_far = String::new();
        for piece in &pieces {
            assert!(
                !so_far.contains(piece.text.trim()) || piece.text.trim().is_empty(),
                "{:?} had already been said in {so_far:?}",
                piece.text
            );
            so_far.push_str(&piece.text);
        }
        assert_eq!(
            joined(&pieces),
            "Hôm nay chúng ta họp, về ngân sách, rồi nghỉ."
        );
    }

    /// Most short utterances have no punctuation at all. They must still arrive.
    #[test]
    fn an_utterance_with_no_punctuation_still_arrives_whole() {
        let pieces = run(1, &["Ừ", "Ừ đúng rồi"], "Ừ đúng rồi");
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].text, "Ừ đúng rồi");
        assert!(pieces[0].last);
    }

    /// Japanese and Chinese punctuate with their own marks, and they are the languages where
    /// waiting for the end of the sentence costs most — the verb is at the end.
    #[test]
    fn cjk_punctuation_is_a_boundary_too() {
        let pieces = run(
            1,
            &["今日は予算について、", "今日は予算について、話します"],
            "今日は予算について、話します。",
        );
        assert_eq!(pieces[0].text, "今日は予算について、");
    }

    /// A clause of three characters costs a request and a synthesis to save less time than either
    /// takes. It waits and goes out inside a longer piece.
    #[test]
    fn a_piece_too_short_to_be_worth_a_request_waits_for_company() {
        let pieces = run(
            1,
            &["Ừ,", "Ừ,", "Ừ, tôi đồng ý với con số đó,"],
            "Ừ, tôi đồng ý với con số đó, cảm ơn.",
        );
        assert!(
            pieces.iter().all(|p| p.text != "Ừ,"),
            "a two-character clause went out alone: {pieces:?}"
        );
        assert_eq!(joined(&pieces), "Ừ, tôi đồng ý với con số đó, cảm ơn.");
    }

    /// The honest cost, counted. The model agreed with itself twice and was still wrong.
    #[test]
    fn a_final_that_contradicts_what_was_said_is_counted() {
        let mut committer = Committer::new();
        committer.partial(1, "Chúng ta cần bàn về ngân sách,");
        committer.partial(1, "Chúng ta cần bàn về ngân sách,");
        assert_eq!(committer.revisions(), 0);

        committer.settle(1, "Chúng ta cần bàn về ngân hàng, xong rồi.");
        assert_eq!(committer.revisions(), 1);
    }

    /// And the words already said are not said again, even when the final disagrees.
    #[test]
    fn a_contradicted_tail_is_measured_from_where_they_stop_agreeing() {
        let mut committer = Committer::new();
        committer.partial(1, "Chúng ta cần bàn về ngân sách,");
        let early = committer
            .partial(1, "Chúng ta cần bàn về ngân sách,")
            .unwrap();
        assert_eq!(early.text, "Chúng ta cần bàn về ngân sách,");

        let tail = committer
            .settle(1, "Chúng ta cần bàn về ngân hàng, xong rồi.")
            .unwrap();
        assert!(
            !tail.text.contains("Chúng ta cần bàn về ngân"),
            "the agreed opening was said twice: {:?}",
            tail.text
        );
        assert!(tail.text.ends_with("xong rồi."));
    }

    /// A final can be dropped — the hallucination filter throws some away. The next utterance must
    /// not inherit the last one's state, or the feature stops for the rest of the meeting.
    #[test]
    fn a_new_utterance_without_its_predecessors_final_starts_clean() {
        let mut committer = Committer::new();
        committer.partial(1, "Câu thứ nhất, còn dài");
        committer.partial(1, "Câu thứ nhất, còn dài");

        // Nothing closed utterance 1. Utterance 2 arrives anyway.
        let piece = committer.settle(2, "Câu thứ hai.").unwrap();
        assert_eq!(piece.seq, 2);
        assert_eq!(piece.text, "Câu thứ hai.");
    }

    /// The floor is about speech, not characters, and the two scripts disagree by about three
    /// times. A flat character count rejected a complete Japanese clause and would have committed
    /// nothing at all in the languages that need it most.
    #[test]
    fn a_complete_clause_is_not_rejected_for_being_short_in_japanese() {
        assert!(weight("今日は予算について、") >= MIN_WEIGHT);
        assert!(weight("Ừ,") < MIN_WEIGHT);
        // Vietnamese and English are counted as themselves.
        assert_eq!(weight("ngan sach"), 9);
    }

    /// Multi-byte text must not be cut between the bytes of a character.
    #[test]
    fn the_agreed_prefix_is_cut_where_a_string_can_be_cut() {
        // These differ inside the last character's bytes, not between characters.
        assert_eq!(common_prefix("ngân", "ngâm"), "ngâ".len());
        assert_eq!(common_prefix("今日は", "今日が"), "今日".len());
        assert_eq!(common_prefix("", "gì"), 0);
    }

    /// An empty final closes the utterance rather than leaving it open forever.
    #[test]
    fn a_final_adding_nothing_still_says_the_utterance_is_over() {
        let mut committer = Committer::new();
        committer.partial(1, "Xong rồi, thế thôi nhé.");
        committer.partial(1, "Xong rồi, thế thôi nhé.");
        let last = committer.settle(1, "Xong rồi, thế thôi nhé.").unwrap();
        assert!(last.last);
    }
}
