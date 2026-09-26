//! Translating a meeting while it is still happening.
//!
//! This is the whole "watch a talk in another language" feature. There is no separate window and no
//! YouTube integration, because there is nothing to integrate with: the system-audio loopback
//! already captures whatever is playing, the pipeline already transcribes it, and what was missing
//! was translating the lines as they land. Turn on system audio, turn on live translation, press
//! play on anything.
//!
//! Four things this has to get right.
//!
//! **It never touches the decode path.** A translation is a network round trip measured in seconds;
//! the audio loop runs on a 30 ms budget. Segments are handed to a background task and the result
//! comes back as a separate [`Event::Translation`] keyed by `seq`. The transcript is never late even
//! when the subtitle is.
//!
//! **It batches, but not for long.** One request per line is slow and expensive, and a model
//! translating a sentence with no context around it loses pronouns — which in Vietnamese is most of
//! the meaning. Waiting for a full batch would mean subtitles a minute behind. So a batch flushes on
//! whichever comes first: enough lines, or enough time.
//!
//! **It drops rather than falls behind.** If the model is slower than the speaker, a queue grows
//! without bound and every subtitle drifts further from the audio. A subtitle three minutes late is
//! worse than no subtitle, so the queue is capped and the oldest lines are dropped with a count the
//! user can see.
//!
//! **A failed translation costs a line, not the meeting.** Recording continues regardless; the
//! error is reported once and the next batch is attempted.

use std::collections::VecDeque;

use summo_core::{Event, Result};
use summo_llm::prompt;

use crate::translate::Translator;

/// Lines per request.
///
/// Smaller than the offline batch of 25: this trades context for latency, and eight lines is
/// roughly twenty seconds of speech — enough for the model to see who is talking to whom, short
/// enough that a subtitle is not embarrassingly late.
pub const BATCH: usize = 8;

/// Longest a line waits for company before its batch goes anyway.
///
/// A pause in the conversation must not strand the sentence before it. Four seconds is under the
/// point where a viewer starts looking for the subtitle that is not there.
pub const MAX_WAIT_MS: u64 = 4_000;

/// Lines allowed to queue before the oldest are dropped.
///
/// Two batches' worth. Past this, the model is losing to the speaker and no amount of queueing wins
/// it back — dropping is what keeps the remaining subtitles near the audio.
pub const MAX_QUEUE: usize = BATCH * 2;

/// One line waiting to be translated.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub seq: u64,
    pub text: String,
    /// What language it was spoken in, when the decoder or the manifest said.
    ///
    /// Carried because the target is decided per line, not per batch: a line already in the
    /// language it would be translated into is not translated into it. `None` is every line
    /// recorded before `Segment::language` existed and every model that reports nothing and
    /// declares nothing — those are translated into everything asked for, which is what the
    /// feature did for all of them until now.
    pub language: Option<String>,
}

/// Collects lines and decides when to send them.
///
/// Pure logic, no clock and no client of its own: the caller supplies how long the oldest line has
/// waited, which is what makes every rule here testable without sleeping.
#[derive(Debug, Default)]
pub struct Batcher {
    queue: VecDeque<Pending>,
    /// Lines thrown away because the model could not keep up, since the last time it was reported.
    dropped: usize,
}

impl Batcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a line. Returns `false` if an older line had to be dropped to make room.
    ///
    /// Blank lines are not queued at all: the recogniser emits them on a cough, and paying for a
    /// translation request to render an empty subtitle is the worst trade in this module.
    pub fn push(&mut self, seq: u64, text: &str, language: Option<String>) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return true;
        }

        self.queue.push_back(Pending {
            seq,
            text: text.to_string(),
            language,
        });

        if self.queue.len() > MAX_QUEUE {
            // Oldest first: the newest line is the one nearest the audio the viewer is hearing.
            self.queue.pop_front();
            self.dropped += 1;
            return false;
        }
        true
    }

    /// Whether to send now, given how long the oldest queued line has been waiting and whether
    /// anything is already out.
    ///
    /// `idle` is the case this was missing, and it is the common one. Batching trades latency for
    /// context, and the trade is worth making *while a request is in flight*: those lines are
    /// waiting on the model anyway, so grouping them costs nothing and buys the model a view of
    /// who is talking to whom.
    ///
    /// With nothing in flight it buys the same context and costs the wait outright. In an ordinary
    /// conversation — one sentence, a pause, another sentence — a batch never fills, so every line
    /// sat here for the full [`MAX_WAIT_MS`] before the request was even sent, and then waited for
    /// the model on top. Four seconds plus a round trip, for a subtitle, every time.
    ///
    /// Reported as "phần dịch chậm quá, phải mấy s sau khi nói". The comment on `MAX_WAIT_MS` said
    /// four seconds was "under the point where a viewer starts looking for the subtitle that is not
    /// there". That was a guess about somebody else's patience, and it was wrong.
    ///
    /// So: first line goes at once, and anything that arrives while it is out rides the next batch.
    /// Latency when idle is the model alone; throughput under load is unchanged.
    #[must_use]
    pub fn ready(&self, waited_ms: u64, idle: bool) -> bool {
        !self.queue.is_empty() && (idle || self.queue.len() >= BATCH || waited_ms >= MAX_WAIT_MS)
    }

    /// Take up to one batch.
    pub fn take(&mut self) -> Vec<Pending> {
        let n = self.queue.len().min(BATCH);
        self.queue.drain(..n).collect()
    }

    /// Take everything, for the end of a session.
    pub fn drain(&mut self) -> Vec<Pending> {
        self.queue.drain(..).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// How many lines have been dropped, clearing the count.
    ///
    /// Reported rather than logged: a viewer whose subtitles have gaps should be told the machine
    /// could not keep up, not left to assume the speaker said nothing.
    pub fn take_dropped(&mut self) -> usize {
        std::mem::take(&mut self.dropped)
    }
}

/// Translate one batch and turn it into events.
///
/// A line the model did not return produces no event at all, rather than an empty one. The
/// interface leaves the original text in place, which is a worse subtitle than a translation and a
/// much better one than a blank.
pub async fn translate_batch(
    translator: &Translator,
    batch: &[Pending],
    lang: &str,
    glossary: &prompt::Glossary,
) -> Result<Vec<Event>> {
    if batch.is_empty() {
        return Ok(Vec::new());
    }

    let lines: Vec<&str> = batch.iter().map(|p| p.text.as_str()).collect();
    // Which prompt this becomes is the translator's decision, not this function's. Live subtitles
    // and a whole-meeting translation must not be able to disagree about it: a dedicated
    // translation model handed the numbered-batch prompt does not translate at all.
    let (parsed, _requests) = translator.run(&lines, lang, glossary).await?;

    Ok(pair(batch, &parsed, lang))
}

/// Where a run of finished translations goes.
///
/// A channel rather than a return value, because the two shapes below differ in *when* they answer
/// and a function that returns once cannot express "the first line, now".
pub type Sink = tokio::sync::mpsc::UnboundedSender<Vec<Event>>;

/// One request per target, the whole run in it.
///
/// The right shape when grouping buys context or concurrency — see [`Translator::batching_helps`].
/// Everything is sent together at the end because it all finishes together anyway.
///
/// Public for the same reason [`translate_batch`] and [`pair`] are: the two shapes differ in *when*
/// they answer, which is the whole point of them, and a test that cannot drive each one separately
/// cannot tell them apart.
pub async fn grouped(
    translator: &Translator,
    batch: &[Pending],
    langs: &[String],
    glossary: &prompt::Glossary,
    tx: &Sink,
) {
    let mut events = Vec::new();
    for lang in langs {
        // Per target, because the answer differs per target: on a Vietnamese-and-English meeting
        // with both chosen, the Vietnamese lines of this batch belong to the English pass and none
        // of the Vietnamese one. Filtering the batch before it is queued would need one queue per
        // language for the same lines.
        let mine = for_target(batch, lang);
        if mine.is_empty() {
            // Nothing said, deliberately. This is the batch where everybody was already speaking
            // the language somebody asked for — the "nothing happened" that the note under the
            // control exists to explain, and a per-batch notice for it would fire on every sentence
            // of a monolingual meeting.
            continue;
        }
        match translate_batch(translator, &mine, lang, glossary).await {
            Ok(mut translated) => events.append(&mut translated),
            Err(e) => events.push(failure(lang, &e)),
        }
    }
    let _ = tx.send(events);
}

/// One request per line, answered as each line comes back.
///
/// For a backend that gains nothing from grouping, holding the first answer until the last line is
/// decoded is a wait bought with nothing. Eight lines at 241 ms is 1.9 seconds of it per target.
///
/// **Lines outside, languages inside.** The other nesting finishes every subtitle in the first
/// language before starting the second, which makes the second reader wait out the whole batch for
/// their first line. This way each line is finished in every language before the next line starts,
/// so both readers get line one at about the same moment.
///
/// One send per line rather than per (line, target): the difference is one decode, and a reader
/// seeing their two subtitles appear together is worth more than saving it.
pub async fn line_by_line(
    translator: &Translator,
    batch: &[Pending],
    langs: &[String],
    glossary: &prompt::Glossary,
    tx: &Sink,
) {
    // A target whose model has no token for it fails on every line. Reported once for the run, the
    // same as `grouped` reports it once for the request — eight identical errors for one broken
    // target is the interface shouting a fault it already stated.
    let mut reported: Vec<&str> = Vec::new();

    for pending in batch {
        let one = std::slice::from_ref(pending);
        let mut events = Vec::new();
        for lang in langs {
            if for_target(one, lang).is_empty() {
                continue;
            }
            match translate_batch(translator, one, lang, glossary).await {
                Ok(mut translated) => events.append(&mut translated),
                Err(e) => {
                    if !reported.contains(&lang.as_str()) {
                        reported.push(lang.as_str());
                        events.push(failure(lang, &e));
                    }
                }
            }
        }
        // Nothing for this line in any target — every one of them was already in the language
        // asked for. Sending an empty vector would wake the socket loop to deliver nothing.
        if !events.is_empty() {
            let _ = tx.send(events);
        }
    }
}

/// One failed target costs its subtitles, not the other targets and not the recording.
///
/// Transient, so the interface can say so without stopping anything — and so a language whose model
/// has no token for it does not take the working one down with it.
fn failure(lang: &str, error: &summo_core::Error) -> Event {
    Event::Error {
        message: format!("không dịch được sang {lang}: {error}"),
        transient: true,
        code: None,
    }
}

/// The lines of a batch that `lang` is actually a translation for.
///
/// Two languages has not meant "into two languages" since the offline pass learned to skip a line
/// already in the language it would be translated into. The live path never learned it: it sent
/// every line to every target, so a bilingual meeting with both languages chosen put a Vietnamese
/// "translation" of each Vietnamese line under it, and paid a request for it. The rule was written,
/// shipped, described in the interface — and read on one of the two paths that needed it.
///
/// Separate from [`Batcher`] on purpose. One line can be wanted by one target and not another, so
/// the queue holds every line and the decision is made where the request is: filtering on the way
/// in would mean a queue per language, holding the same lines, flushing on different clocks.
#[must_use]
pub fn for_target(batch: &[Pending], lang: &str) -> Vec<Pending> {
    batch
        .iter()
        .filter(|p| !crate::translate::same_language(p.language.as_deref(), lang))
        .cloned()
        .collect()
}

/// Match a parsed response back to the sequence numbers it belongs to.
///
/// Split out from the request so the alignment — the part that silently corrupts everything if it
/// is wrong — is testable without a model.
#[must_use]
pub fn pair(batch: &[Pending], parsed: &[Option<String>], lang: &str) -> Vec<Event> {
    batch
        .iter()
        .zip(parsed)
        .filter_map(|(pending, text)| {
            let text = text.as_ref()?.trim();
            (!text.is_empty()).then(|| Event::Translation {
                seq: pending.seq,
                lang: lang.to_string(),
                text: text.to_string(),
            })
        })
        .collect()
}

/// What the user turned on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveConfig {
    /// Target language tags. Empty means live translation is off.
    ///
    /// A list, because a meeting can have more than one reader and the second one is nearly free.
    /// SMALL100 is a single multilingual model and the target language is a token it is started
    /// with, so translating a batch into Japanese as well as English is another pass through
    /// weights that are already resident — CPU, not another six hundred megabytes. Assuming
    /// otherwise is why this was one language for as long as it was.
    pub langs: Vec<String>,
    pub glossary: prompt::Glossary,
}

impl LiveConfig {
    #[must_use]
    pub fn enabled(&self) -> bool {
        !self.langs.is_empty()
    }
}

/// A live translation running alongside one recording.
///
/// Holds the queue, spawns the requests, and hands back whatever has come home. The socket loop
/// calls [`LiveTranslator::offer`] with each batch of pipeline events and forwards the result — so
/// translations ride the same connection as the transcript, and a client that disconnects stops
/// paying for them at the next batch.
pub struct LiveTranslator {
    batcher: Batcher,
    /// Lines that were already finished when translation was turned on.
    ///
    /// Turning it on used to mean "from the next sentence", and the interface had no way to say so
    /// — the banner reported that translation was on over a transcript with nothing translated in
    /// it, which reads as broken and was reported as broken. The objection on `Command::Translate`
    /// is about *retranslating*: rewriting text somebody has been reading. A subtitle is added
    /// beside the line, not over it, so there is nothing to rewrite.
    ///
    /// Kept apart from `batcher` rather than pushed into it. The batcher is bounded and drops its
    /// *oldest* line under pressure, which is right for live speech and exactly backwards for a
    /// backlog — a whole meeting pushed through it would keep the last sixteen lines and discard
    /// the rest. This drains only into space the live path is not using.
    backlog: VecDeque<Pending>,
    /// When the oldest queued line arrived, for the deadline.
    since: Option<std::time::Instant>,
    translator: std::sync::Arc<Translator>,
    config: LiveConfig,
    tx: tokio::sync::mpsc::UnboundedSender<Vec<Event>>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<Event>>,
    /// Requests currently out. Capped so a stalled model cannot spawn one task per batch forever.
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// Requests allowed to be outstanding at once.
///
/// Two: enough that a slow response does not stall the next batch, few enough that a model timing
/// out at thirty seconds does not accumulate a dozen doomed requests and a bill to match.
pub const MAX_IN_FLIGHT: usize = 2;

impl LiveTranslator {
    #[must_use]
    pub fn new(translator: Translator, config: LiveConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            batcher: Batcher::new(),
            backlog: VecDeque::new(),
            since: None,
            translator: std::sync::Arc::new(translator),
            config,
            tx,
            rx,
            in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[must_use]
    pub fn languages(&self) -> &[String] {
        &self.config.langs
    }

    /// Feed the pipeline's events in; get translations and notices out.
    ///
    /// Only `Final` segments are queued. Translating a partial would mean paying for a sentence
    /// that is about to change, and showing a subtitle for words the speaker has not finished
    /// saying.
    pub fn offer(&mut self, events: &[Event]) -> Vec<Event> {
        use std::sync::atomic::Ordering;

        for event in events {
            let Event::Final(segment) = event else {
                continue;
            };
            if self.since.is_none() {
                self.since = Some(std::time::Instant::now());
            }
            self.batcher
                .push(segment.seq, &segment.text, segment.language.clone());
        }

        let waited = self.since.map_or(0, |t| t.elapsed().as_millis() as u64);
        let out = self.in_flight.load(Ordering::Relaxed);
        if self.batcher.ready(waited, out == 0) && out < MAX_IN_FLIGHT {
            self.dispatch();
        }

        // The backlog, only into room the live path is not using. Speech happening now outranks
        // speech from a minute ago every time: a subtitle that arrives after the speaker has moved
        // on is the failure this module already refuses to accept for live lines, and filling in
        // the past must not cause it.
        // `== 0`, not `< MAX_IN_FLIGHT`. A slot is kept free for speech that has not happened yet.
        //
        // The live queue emptying used to mean nothing was being translated; now it means the line
        // was sent *immediately*, which is the whole point of the change above. Letting the backlog
        // take the remaining slot on the strength of an empty queue would put a minute-old sentence
        // in front of the one being spoken — the exact failure this module refuses, arriving by a
        // new route.
        if self.batcher.is_empty() && self.in_flight.load(Ordering::Relaxed) == 0 {
            let take = self.backlog.len().min(BATCH);
            if take > 0 {
                let batch: Vec<Pending> = self.backlog.drain(..take).collect();
                self.spawn(batch);
            }
        }

        self.collect()
    }

    /// Take the lines that were already finished when translation was turned on.
    ///
    /// Oldest first, so a meeting fills in from the top the way somebody reads it. Blank lines are
    /// skipped for the same reason the batcher skips them: paying for a request to render an empty
    /// subtitle is the worst trade in this module.
    pub fn backfill(&mut self, lines: impl IntoIterator<Item = (u64, String, Option<String>)>) {
        for (seq, text, language) in lines {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            self.backlog.push_back(Pending {
                seq,
                text: text.to_string(),
                language,
            });
        }
    }

    /// How many old lines are still waiting, for the interface to say so.
    #[must_use]
    pub fn backlog_len(&self) -> usize {
        self.backlog.len()
    }

    /// Send everything still queued, at the end of a session.
    pub fn finish(&mut self) {
        let batch = self.batcher.drain();
        self.spawn(batch);
    }

    fn dispatch(&mut self) {
        let batch = self.batcher.take();
        self.since = if self.batcher.is_empty() {
            None
        } else {
            // The next line already waiting starts its own clock now, rather than inheriting the
            // deadline of the batch that just went — otherwise every later batch fires instantly.
            Some(std::time::Instant::now())
        };
        self.spawn(batch);
    }

    fn spawn(&self, batch: Vec<Pending>) {
        use std::sync::atomic::Ordering;

        if batch.is_empty() {
            return;
        }
        let translator = self.translator.clone();
        let langs = self.config.langs.clone();
        let glossary = self.config.glossary.clone();
        let tx = self.tx.clone();
        let in_flight = self.in_flight.clone();

        // One task for the whole batch, targets done one after another inside it.
        //
        // Not one task per language, which was the obvious shape and the wrong one twice over. The
        // in-process translator is CPU-bound, so two concurrent passes share the same cores and
        // finish no sooner while doubling the peak; and `MAX_IN_FLIGHT` counts outstanding
        // *requests*, so a task per language would quietly halve the number of batches allowed in
        // the air the moment somebody added a second subtitle.
        in_flight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            // How many lines actually travelled together, and which shape carried them.
            //
            // The whole argument about batching turns on this number and nothing reported it. The
            // batch size a user really gets depends on whether the model is losing to the speaker,
            // which is a property of their machine and their meeting — so reasoning about it from
            // the constants is guessing, and this is the line that ends the guess.
            tracing::debug!(
                lines = batch.len(),
                targets = langs.len(),
                grouped = translator.batching_helps(),
                "translating a run"
            );

            // Which shape depends on what grouping is worth here, which is a fact about the
            // backend. See `Translator::batching_helps`.
            if translator.batching_helps() {
                grouped(&translator, &batch, &langs, &glossary, &tx).await;
            } else {
                line_by_line(&translator, &batch, &langs, &glossary, &tx).await;
            }
            in_flight.fetch_sub(1, Ordering::Relaxed);
        });
    }

    /// Whatever has come back since the last call, plus a notice if lines were dropped.
    fn collect(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(events) = self.rx.try_recv() {
            out.extend(events);
        }

        let dropped = self.batcher.take_dropped();
        if dropped > 0 {
            out.push(Event::info(format!(
                "bỏ {dropped} câu dịch — mô hình không theo kịp"
            )));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable_provider() -> summo_llm::Provider {
        // Nothing listens on port 1; a dispatched request fails fast and comes back as an error
        // event, which is exactly the signal these tests need.
        summo_llm::Provider::custom("x", "http://127.0.0.1:1", "m")
    }

    fn translator(langs: &[&str]) -> LiveTranslator {
        LiveTranslator::new(
            Translator::chat(unreachable_provider()).unwrap(),
            LiveConfig {
                langs: langs.iter().map(|l| (*l).to_string()).collect(),
                glossary: prompt::Glossary::default(),
            },
        )
    }

    fn final_of(seq: u64, text: &str) -> Event {
        Event::Final(summo_core::segment::Segment::new(
            seq,
            summo_core::segment::Lane::System,
            text,
            0.0,
            1.0,
        ))
    }

    /// Translating a partial means paying for a sentence that is about to change, and showing a
    /// subtitle for words the speaker has not finished saying.
    #[tokio::test]
    async fn only_final_segments_are_queued() {
        let mut live = translator(&["en"]);
        let partial = Event::Partial(summo_core::segment::Segment::new(
            1,
            summo_core::segment::Lane::System,
            "đang nói",
            0.0,
            1.0,
        ));
        live.offer(&[partial]);
        assert!(
            live.batcher.is_empty(),
            "a partial must not become a request"
        );
        assert_eq!(live.in_flight.load(std::sync::atomic::Ordering::Relaxed), 0);

        // The final does. It leaves the queue immediately now rather than waiting for company —
        // see `Batcher::ready` — so what proves it was taken is the request, not the backlog.
        live.offer(&[final_of(2, "xong rồi")]);
        assert_eq!(live.in_flight.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    /// The first line goes at once; the rest ride behind it.
    ///
    /// Waiting for company is free while a request is already out — those lines are waiting on the
    /// model anyway — and costs the wait outright when nothing is. An ordinary conversation never
    /// fills a batch, so the old unconditional deadline charged every line four seconds before the
    /// request was even sent. Reported as "phần dịch chậm quá, phải mấy s sau khi nói".
    #[tokio::test]
    async fn the_first_line_is_sent_at_once_and_the_rest_wait_behind_it() {
        use std::sync::atomic::Ordering;
        let mut live = translator(&["en"]);

        live.offer(&[final_of(0, "câu")]);
        assert_eq!(
            live.in_flight.load(Ordering::Relaxed),
            1,
            "sent immediately"
        );
        assert!(live.batcher.is_empty());

        // Two more while that one is out. `MAX_IN_FLIGHT` is 2, so the second batch goes and the
        // third line waits — which is the trade working as intended rather than a deadline.
        live.offer(&[final_of(1, "câu")]);
        live.offer(&[final_of(2, "câu")]);
        assert!(live.in_flight.load(Ordering::Relaxed) <= MAX_IN_FLIGHT);
    }

    /// The wiring test: a full batch dispatches, the request fails against a dead port, and the
    /// failure comes back as a transient error rather than vanishing or panicking.
    #[tokio::test]
    async fn a_full_batch_dispatches_and_a_failure_is_reported_as_transient() {
        let mut live = translator(&["en"]);
        // Handed over together, the way the pipeline delivers a burst. One at a time would now
        // dispatch the first line on its own — see `the_first_line_is_sent_at_once…` — and this
        // test is about what happens to a *full* batch.
        let burst: Vec<Event> = (0..BATCH).map(|i| final_of(i as u64, "câu")).collect();
        live.offer(&burst);
        assert!(live.batcher.is_empty(), "the batch left the queue");

        // Poll until the spawned request has failed and posted its result.
        let mut reported = Vec::new();
        for _ in 0..80 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            reported.extend(live.offer(&[]));
            if !reported.is_empty() {
                break;
            }
        }

        assert!(
            reported.iter().any(|e| matches!(
                e,
                Event::Error {
                    transient: true,
                    ..
                }
            )),
            "expected a transient failure, got {reported:?}"
        );
    }

    /// The last few lines of a meeting must not be stranded in a batch that never fills.
    #[tokio::test]
    async fn finishing_sends_whatever_is_left() {
        let mut live = translator(&["en"]);
        // Filled past what can be in flight, so something is genuinely left behind to finish.
        for i in 0..(BATCH * MAX_IN_FLIGHT + 2) {
            live.offer(&[final_of(i as u64, "câu cuối")]);
        }
        assert!(!live.batcher.is_empty(), "something is waiting to be sent");

        live.finish();
        assert!(live.batcher.is_empty());
    }

    #[tokio::test]
    async fn the_target_languages_are_the_ones_the_session_asked_for() {
        assert_eq!(translator(&["ja"]).languages(), ["ja"]);
        assert_eq!(translator(&["ja", "en"]).languages(), ["ja", "en"]);
    }

    fn pending(seq: u64, text: &str) -> Pending {
        Pending {
            seq,
            text: text.into(),
            language: None,
        }
    }

    /// The same, from a speaker whose language the decoder reported.
    fn spoken(seq: u64, text: &str, language: &str) -> Pending {
        Pending {
            seq,
            text: text.into(),
            language: Some(language.into()),
        }
    }

    #[test]
    fn a_batch_goes_once_it_is_full() {
        let mut b = Batcher::new();
        for i in 0..BATCH {
            b.push(i as u64, "câu", None);
        }
        assert!(b.ready(0, false), "full: send without waiting");
        assert_eq!(b.take().len(), BATCH);
    }

    /// A pause in the conversation must not strand the sentence before it — and while a request is
    /// already out, waiting for company is free, because those lines are waiting on the model
    /// anyway.
    #[test]
    fn a_lone_line_waits_for_company_only_while_something_is_in_flight() {
        let mut b = Batcher::new();
        b.push(1, "xin chào", None);
        assert!(!b.ready(MAX_WAIT_MS - 1, false));
        assert!(b.ready(MAX_WAIT_MS, false));
    }

    /// The case the deadline was costing four seconds for nothing.
    ///
    /// One sentence, a pause, another sentence — an ordinary conversation — never fills a batch,
    /// so every line sat for the full `MAX_WAIT_MS` before the request was even sent, and then
    /// waited for the model on top. With nothing in flight there is no company coming and nothing
    /// to gain by waiting for it.
    #[test]
    fn the_first_line_after_a_pause_goes_immediately() {
        let mut b = Batcher::new();
        b.push(1, "xin chào", None);
        assert!(
            b.ready(0, true),
            "nothing is in flight, so this line is waiting for company that is not coming"
        );
    }

    #[test]
    fn an_empty_queue_never_sends_a_request() {
        let b = Batcher::new();
        assert!(!b.ready(0, false));
        assert!(!b.ready(MAX_WAIT_MS * 10, false));
        // Not even when idle: there is nothing to send.
        assert!(!b.ready(0, true));
    }

    /// The recogniser emits blank finals on a cough. Paying for a request to render an empty
    /// subtitle is the worst trade available here.
    #[test]
    fn a_blank_line_is_not_queued_at_all() {
        let mut b = Batcher::new();
        b.push(1, "   ", None);
        b.push(2, "", None);
        assert!(b.is_empty());
    }

    /// If the model loses to the speaker, a growing queue makes every later subtitle worse. Three
    /// minutes late is worse than absent.
    #[test]
    fn the_oldest_lines_are_dropped_when_the_model_falls_behind() {
        let mut b = Batcher::new();
        for i in 0..MAX_QUEUE {
            assert!(b.push(i as u64, "câu", None), "still room at {i}");
        }
        assert!(
            !b.push(999, "mới", None),
            "the overflowing push reports the drop"
        );

        assert_eq!(b.len(), MAX_QUEUE);
        assert_eq!(b.take_dropped(), 1);
        assert_eq!(b.take_dropped(), 0, "the count clears when it is read");

        // The newest line survived; the oldest did not.
        let batch = b.take();
        assert_ne!(batch[0].seq, 0);
    }

    #[test]
    fn taking_a_batch_leaves_the_rest_queued() {
        let mut b = Batcher::new();
        for i in 0..BATCH + 3 {
            b.push(i as u64, "câu", None);
        }
        assert_eq!(b.take().len(), BATCH);
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn draining_takes_everything_for_the_end_of_a_session() {
        let mut b = Batcher::new();
        for i in 0..BATCH + 3 {
            b.push(i as u64, "câu", None);
        }
        assert_eq!(b.drain().len(), BATCH + 3);
        assert!(b.is_empty());
    }

    /// The bug this guards: a dropped line shifting every later translation onto the wrong
    /// sentence. `parse_translation` returns `None` in place, and `pair` has to keep it there.
    #[test]
    fn a_line_the_model_skipped_does_not_shift_the_others() {
        let batch = [pending(10, "một"), pending(11, "hai"), pending(12, "ba")];
        let parsed = [Some("one".into()), None, Some("three".into())];

        let events = pair(&batch, &parsed, "en");
        assert_eq!(
            events,
            vec![
                Event::Translation {
                    seq: 10,
                    lang: "en".into(),
                    text: "one".into()
                },
                Event::Translation {
                    seq: 12,
                    lang: "en".into(),
                    text: "three".into()
                },
            ]
        );
    }

    #[test]
    fn a_translation_that_came_back_blank_produces_no_event() {
        let batch = [pending(1, "một")];
        assert!(pair(&batch, &[Some("   ".into())], "en").is_empty());
    }

    /// A short response must not panic on the longer batch it was meant to answer.
    #[test]
    fn a_response_with_fewer_lines_than_asked_for_is_survivable() {
        let batch = [pending(1, "một"), pending(2, "hai")];
        assert_eq!(pair(&batch, &[Some("one".into())], "en").len(), 1);
    }

    /// The headline case, and the one that was wrong live while being right offline.
    ///
    /// A Vietnamese company on a call with an English customer, both languages chosen. Each line
    /// belongs to exactly one of the two passes: the Vietnamese ones are what the English subtitle
    /// is for, and the English ones are what the Vietnamese subtitle is for. Before this, every
    /// line went to both — so every Vietnamese line got a Vietnamese "translation" of itself
    /// underneath it, at the price of a request.
    #[test]
    fn a_line_is_not_translated_into_the_language_it_is_already_in() {
        let batch = [
            spoken(1, "Xin chào các bạn", "vi"),
            spoken(2, "Thanks for having me", "en"),
        ];

        let into_english = for_target(&batch, "en");
        assert_eq!(into_english.len(), 1);
        assert_eq!(into_english[0].seq, 1);

        let into_vietnamese = for_target(&batch, "vi");
        assert_eq!(into_vietnamese.len(), 1);
        assert_eq!(into_vietnamese[0].seq, 2);
    }

    /// Region is a spelling of a language, not a different one. Whisper answers `en-US` on some
    /// builds, and comparing it raw against a target of `en` would translate English into English.
    #[test]
    fn a_region_tag_counts_as_the_language_it_is_a_region_of() {
        assert!(for_target(&[spoken(1, "hello", "en-US")], "en").is_empty());
        assert_eq!(for_target(&[spoken(1, "hello", "en-US")], "vi").len(), 1);
    }

    /// A line nobody labelled is translated into everything asked for.
    ///
    /// That is every line recorded before `Segment::language` existed, and every model that reports
    /// no language and declares none. Reading "unknown" as "already in this language" would turn
    /// live translation off for them, which is a worse failure than a redundant subtitle.
    #[test]
    fn an_unlabelled_line_is_still_translated() {
        assert_eq!(for_target(&[pending(1, "một câu")], "vi").len(), 1);
        assert_eq!(for_target(&[pending(1, "một câu")], "en").len(), 1);
    }

    /// One target and the language everybody is speaking: nothing to send, and nothing sent.
    ///
    /// Somebody picks Vietnamese on a Vietnamese meeting. Nothing happening is the correct outcome
    /// — the note under the control is what explains it — and a batch of zero must not become a
    /// request with an empty prompt.
    #[test]
    fn a_monolingual_meeting_translated_into_its_own_language_asks_for_nothing() {
        let batch = [spoken(1, "một", "vi"), spoken(2, "hai", "vi")];
        assert!(for_target(&batch, "vi").is_empty());
    }

    #[test]
    fn no_language_means_live_translation_is_off() {
        assert!(!LiveConfig::default().enabled());

        let on = LiveConfig {
            langs: vec!["en".into()],
            glossary: prompt::Glossary::default(),
        };
        assert!(on.enabled());
    }

    #[tokio::test]
    async fn an_empty_batch_costs_no_request() {
        // Unreachable on purpose: reaching the model would fail the test.
        let events = translate_batch(
            &Translator::chat(unreachable_provider()).unwrap(),
            &[],
            "en",
            &prompt::Glossary::default(),
        )
        .await
        .expect("no request needed");
        assert!(events.is_empty());
    }
}

#[cfg(test)]
mod backfilling {
    use super::*;

    fn unreachable_provider() -> summo_llm::Provider {
        summo_llm::Provider::custom("x", "http://127.0.0.1:1", "m")
    }

    fn live(langs: &[&str]) -> LiveTranslator {
        LiveTranslator::new(
            Translator::chat(unreachable_provider()).unwrap(),
            LiveConfig {
                langs: langs.iter().map(|l| (*l).to_string()).collect(),
                glossary: prompt::Glossary::default(),
            },
        )
    }

    /// Turning translation on takes the meeting so far with it.
    ///
    /// It used to mean "from the next sentence", with nothing on screen to say so — the banner
    /// reported translation was on over a transcript with nothing translated in it.
    #[test]
    fn the_lines_already_said_are_queued_too() {
        let mut translator = live(&["en"]);
        assert_eq!(translator.backlog_len(), 0);

        translator.backfill([
            (0, "xin chào".to_string(), None),
            (1, "  ".to_string(), None),
            (2, "hai".to_string(), None),
        ]);
        assert_eq!(
            translator.backlog_len(),
            2,
            "a blank line is not worth a request"
        );
    }

    /// And it never delays the sentence being spoken now.
    ///
    /// A subtitle arriving after the speaker has moved on is the failure this module refuses for
    /// live lines; filling in the past must not cause it. So the backlog drains only when the live
    /// batcher is empty — with a line waiting to go, the backlog stays exactly where it is.
    // `tokio::test`, because a live line is now *dispatched* rather than queued and dispatching
    // spawns.
    #[tokio::test]
    async fn live_speech_is_never_held_up_by_the_backlog() {
        let mut translator = live(&["en"]);
        translator.backfill((0..20).map(|seq| (seq, format!("câu {seq}"), None)));
        let before = translator.backlog_len();
        assert_eq!(before, 20);

        // One final arrives. It is dispatched at once rather than queued — see `Batcher::ready` —
        // so what holds the backlog back is the request in flight, not a non-empty queue.
        let segment = summo_core::segment::Segment::new(
            99,
            summo_core::segment::Lane::Mic,
            "đang nói",
            0.0,
            1.0,
        );
        translator.offer(&[Event::Final(segment)]);

        assert_eq!(
            translator.backlog_len(),
            before,
            "the backlog went ahead of the sentence being spoken"
        );
    }
}
