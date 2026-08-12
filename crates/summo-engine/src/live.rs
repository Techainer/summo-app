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
    pub fn push(&mut self, seq: u64, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return true;
        }

        self.queue.push_back(Pending {
            seq,
            text: text.to_string(),
        });

        if self.queue.len() > MAX_QUEUE {
            // Oldest first: the newest line is the one nearest the audio the viewer is hearing.
            self.queue.pop_front();
            self.dropped += 1;
            return false;
        }
        true
    }

    /// Whether to send now, given how long the oldest queued line has been waiting.
    #[must_use]
    pub fn ready(&self, waited_ms: u64) -> bool {
        !self.queue.is_empty() && (self.queue.len() >= BATCH || waited_ms >= MAX_WAIT_MS)
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
#[derive(Debug, Clone, PartialEq)]
pub struct LiveConfig {
    /// Target language tag. Empty means live translation is off.
    pub lang: String,
    pub glossary: prompt::Glossary,
}

impl LiveConfig {
    #[must_use]
    pub fn enabled(&self) -> bool {
        !self.lang.trim().is_empty()
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
            since: None,
            translator: std::sync::Arc::new(translator),
            config,
            tx,
            rx,
            in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[must_use]
    pub fn language(&self) -> &str {
        &self.config.lang
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
            self.batcher.push(segment.seq, &segment.text);
        }

        let waited = self.since.map_or(0, |t| t.elapsed().as_millis() as u64);
        if self.batcher.ready(waited) && self.in_flight.load(Ordering::Relaxed) < MAX_IN_FLIGHT {
            self.dispatch();
        }

        self.collect()
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
        let lang = self.config.lang.clone();
        let glossary = self.config.glossary.clone();
        let tx = self.tx.clone();
        let in_flight = self.in_flight.clone();

        in_flight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let result = translate_batch(&translator, &batch, &lang, &glossary).await;
            in_flight.fetch_sub(1, Ordering::Relaxed);

            let events = match result {
                Ok(events) => events,
                // One failed batch costs those lines, not the recording. Reported once, as a
                // transient error, so the interface can say so without stopping anything.
                Err(e) => vec![Event::Error {
                    message: format!("không dịch được: {e}"),
                    transient: true,
                }],
            };
            let _ = tx.send(events);
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

    fn translator(lang: &str) -> LiveTranslator {
        LiveTranslator::new(
            Translator::chat(unreachable_provider()).unwrap(),
            LiveConfig {
                lang: lang.into(),
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
        let mut live = translator("en");
        let partial = Event::Partial(summo_core::segment::Segment::new(
            1,
            summo_core::segment::Lane::System,
            "đang nói",
            0.0,
            1.0,
        ));
        live.offer(&[partial]);
        assert!(live.batcher.is_empty());

        live.offer(&[final_of(2, "xong rồi")]);
        assert_eq!(live.batcher.len(), 1);
    }

    /// A few lines must not each cost a request; the batch waits for company.
    #[tokio::test]
    async fn a_short_run_of_lines_does_not_dispatch_yet() {
        let mut live = translator("en");
        for i in 0..3 {
            live.offer(&[final_of(i, "câu")]);
        }
        assert_eq!(live.batcher.len(), 3, "still queued, nothing sent");
    }

    /// The wiring test: a full batch dispatches, the request fails against a dead port, and the
    /// failure comes back as a transient error rather than vanishing or panicking.
    #[tokio::test]
    async fn a_full_batch_dispatches_and_a_failure_is_reported_as_transient() {
        let mut live = translator("en");
        for i in 0..BATCH {
            live.offer(&[final_of(i as u64, "câu")]);
        }
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
        let mut live = translator("en");
        live.offer(&[final_of(1, "câu cuối")]);
        assert_eq!(live.batcher.len(), 1);

        live.finish();
        assert!(live.batcher.is_empty());
    }

    #[tokio::test]
    async fn the_target_language_is_the_one_the_session_asked_for() {
        assert_eq!(translator("ja").language(), "ja");
    }

    fn pending(seq: u64, text: &str) -> Pending {
        Pending {
            seq,
            text: text.into(),
        }
    }

    #[test]
    fn a_batch_goes_once_it_is_full() {
        let mut b = Batcher::new();
        for i in 0..BATCH {
            b.push(i as u64, "câu");
        }
        assert!(b.ready(0), "full: send without waiting");
        assert_eq!(b.take().len(), BATCH);
    }

    /// A pause in the conversation must not strand the sentence before it.
    #[test]
    fn a_lone_line_goes_once_it_has_waited_long_enough() {
        let mut b = Batcher::new();
        b.push(1, "xin chào");
        assert!(!b.ready(MAX_WAIT_MS - 1));
        assert!(b.ready(MAX_WAIT_MS));
    }

    #[test]
    fn an_empty_queue_never_sends_a_request() {
        let b = Batcher::new();
        assert!(!b.ready(0));
        assert!(!b.ready(MAX_WAIT_MS * 10));
    }

    /// The recogniser emits blank finals on a cough. Paying for a request to render an empty
    /// subtitle is the worst trade available here.
    #[test]
    fn a_blank_line_is_not_queued_at_all() {
        let mut b = Batcher::new();
        b.push(1, "   ");
        b.push(2, "");
        assert!(b.is_empty());
    }

    /// If the model loses to the speaker, a growing queue makes every later subtitle worse. Three
    /// minutes late is worse than absent.
    #[test]
    fn the_oldest_lines_are_dropped_when_the_model_falls_behind() {
        let mut b = Batcher::new();
        for i in 0..MAX_QUEUE {
            assert!(b.push(i as u64, "câu"), "still room at {i}");
        }
        assert!(!b.push(999, "mới"), "the overflowing push reports the drop");

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
            b.push(i as u64, "câu");
        }
        assert_eq!(b.take().len(), BATCH);
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn draining_takes_everything_for_the_end_of_a_session() {
        let mut b = Batcher::new();
        for i in 0..BATCH + 3 {
            b.push(i as u64, "câu");
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

    #[test]
    fn an_empty_language_means_live_translation_is_off() {
        let off = LiveConfig {
            lang: "  ".into(),
            glossary: prompt::Glossary::default(),
        };
        assert!(!off.enabled());

        let on = LiveConfig {
            lang: "en".into(),
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
