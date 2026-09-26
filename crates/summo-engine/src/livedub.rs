//! Speaking the translation while the meeting is still happening.
//!
//! [`crate::dub`] is the other one, and the difference is not a setting. That one takes a finished
//! meeting and a finished translation, says every line once to find out how long it takes, plans
//! the whole timeline against the real durations, says every line again at the speed its slot
//! needs, and mixes the result over the original. It is the right shape for a file somebody will
//! play later and it cannot be made live: `summo_tts::plan` has to see the whole meeting before it
//! can place the first line.
//!
//! Live, there is no timeline to fit into. There is a person with headphones on who is behind the
//! speaker and wants to be less behind. So: one pass, one line at a time, and a running controller
//! in place of a plan.
//!
//! ## Where the time goes
//!
//! Measured on this machine, for a line that has just been recognised:
//!
//! ```text
//! translation (SMALL100, one line)      241 ms
//! synthesis   (VITS piper, RTF 0.065)   ~200 ms for a three-second line
//! ```
//!
//! Loading the voice is 1.7–1.9 s and is not in that list, because [`crate::tts_warm`] pays it when
//! the listener chooses a language rather than when the first line arrives.
//!
//! ## Falling behind
//!
//! Dubbed speech is not the same length as what it translates, so a listener drifts. The offline
//! planner solves this globally; live, the only thing known is how much audio has been sent that
//! has not finished playing yet — [`Pace`] turns that into a speed, and past the point where
//! speeding up stops being listenable it turns it into a dropped line instead.
//!
//! Dropping rather than queueing is the same judgement [`crate::live`] makes about subtitles, for
//! the same reason: a line spoken a minute after it was said is worse than a line not spoken, and
//! a queue that only grows never recovers. The count is reported so the interface can say so.
//!
//! ## What is not here yet
//!
//! This speaks **finished translations** — it watches for [`Event::Translation`] and synthesises
//! it. That makes a live dub arrive about half a second after the line settles, which is the
//! subtitle's own latency plus synthesis.
//!
//! It does not yet speak *clauses*, which is what [`crate::commit`] was built for and what would
//! remove the 400 ms the gate spends waiting for trailing silence. That module is tested and
//! unwired; joining the two is the next step, not something this one quietly half-does.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use summo_core::event::Event;
use summo_tts::Synthesizer;

/// Requests allowed to be outstanding at once.
///
/// One. Synthesis is CPU-bound and the voice is a single ONNX session behind a lock, so a second
/// concurrent request would wait for the first and then contend for the same cores — the mistake
/// [`crate::live`] documents for its own spawn shape. With a real-time factor of 0.065 there is no
/// throughput to win here anyway.
const MAX_IN_FLIGHT: usize = 1;

/// Backlog below which nothing is hurried.
///
/// Some backlog is wanted: it is what stops the dub stuttering between lines. A second is about one
/// short sentence in hand.
pub const EASY_S: f64 = 1.0;

/// Backlog past which a line is dropped rather than spoken late.
///
/// Four seconds behind is where a listener stops being able to match the dub to the room. It is the
/// same order as [`crate::live::MAX_QUEUE`]'s two batches and chosen on the same grounds: past it,
/// the machine has lost to the speaker and no amount of queueing wins it back.
pub const DROP_S: f64 = 4.0;

/// Fastest a line is allowed to be spoken.
///
/// The offline planner's own ceiling, and for the reason given there: past about a third faster,
/// speech stops being comfortable and the listener loses more to effort than they gain to time.
pub const MAX_SPEED: f64 = 1.35;

/// What to do with a line, given how far behind the dub already is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pace {
    /// Say it, at this speed. `1.0` is the voice's natural pace.
    Speak(f64),
    /// Do not say it. Speaking it would put the listener further behind than it is worth.
    Skip,
}

/// Choose a pace from the backlog.
///
/// Pure, so the rule can be argued with and tested without a clock, a model or a sound card — the
/// same reason [`crate::live::Batcher`] takes the elapsed time as an argument.
///
/// Below [`EASY_S`] nothing is hurried. From there the speed rises smoothly to [`MAX_SPEED`] at
/// [`DROP_S`], and past that the line is skipped. Smoothly rather than in steps: a dub that
/// switches between two speeds mid-conversation sounds like a fault, and a listener notices the
/// change far more than the pace.
#[must_use]
pub fn pace(backlog_s: f64) -> Pace {
    if !backlog_s.is_finite() || backlog_s <= EASY_S {
        return Pace::Speak(1.0);
    }
    if backlog_s >= DROP_S {
        return Pace::Skip;
    }
    let through = (backlog_s - EASY_S) / (DROP_S - EASY_S);
    Pace::Speak(1.0 + through * (MAX_SPEED - 1.0))
}

/// One piece of dubbed audio, ready to be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// The utterance this speaks, so a client can line it up with the transcript.
    pub seq: u64,
    /// The language being spoken.
    pub lang: String,
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Chunk {
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.rate)
    }

    /// Pack for the socket.
    ///
    /// Mirrors the interface's `encodeFrame`, which sends a lane tag byte and little-endian `f32`
    /// samples the other way. The tag distinguishes this from anything else binary the daemon may
    /// send later; the language is in the frame rather than assumed from the session, because a
    /// session can have two subtitle languages and will one day be able to dub into a chosen one.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let lang = self.lang.as_bytes();
        let mut out = Vec::with_capacity(10 + lang.len() + self.samples.len() * 4);
        out.push(TAG_DUB);
        // A language tag that does not fit in a byte is not a language tag. Truncating beats
        // refusing to send the audio, and beats a length field four times the size of the string.
        let len = lang.len().min(u8::MAX as usize);
        out.push(len as u8);
        out.extend_from_slice(&lang[..len]);
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        for sample in &self.samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    /// Unpack one. `None` for anything that is not a well-formed dub frame.
    ///
    /// Exists so the format has a test rather than two hand-written halves that drift — the
    /// interface's decoder is the other half and is checked against the bytes this produces.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&tag, rest) = bytes.split_first()?;
        if tag != TAG_DUB {
            return None;
        }
        let (&len, rest) = rest.split_first()?;
        let len = len as usize;
        if rest.len() < len + 12 {
            return None;
        }
        let lang = std::str::from_utf8(&rest[..len]).ok()?.to_string();
        let rest = &rest[len..];
        let rate = u32::from_le_bytes(rest[..4].try_into().ok()?);
        let seq = u64::from_le_bytes(rest[4..12].try_into().ok()?);
        let body = &rest[12..];
        if !body.len().is_multiple_of(4) {
            return None;
        }
        let samples = body
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Some(Self {
            seq,
            lang,
            rate,
            samples,
        })
    }
}

/// First byte of a dub frame on the socket.
pub const TAG_DUB: u8 = 0x01;

/// A live dub running alongside one recording.
///
/// The same shape as [`crate::live::LiveTranslator`], deliberately: the socket loop calls
/// [`LiveDub::offer`] with each batch of pipeline events and forwards what comes back, so a client
/// that disconnects stops paying for synthesis at the next batch.
pub struct LiveDub {
    lang: String,
    /// The voice, behind a lock because it crosses into a blocking thread and back.
    voice: Arc<Mutex<Box<dyn Synthesizer>>>,
    tx: tokio::sync::mpsc::UnboundedSender<Chunk>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Chunk>,
    in_flight: Arc<AtomicUsize>,
    /// When everything already handed over will have finished playing.
    ///
    /// The daemon cannot know what the client has played, and asking would be a round trip per
    /// chunk. What it does know is how much audio it has sent and how long that takes to say, which
    /// is the same number as long as the client plays back to back — which is what a dub is.
    speaks_until: Option<Instant>,
    /// Lines not spoken because the dub had fallen too far behind.
    skipped: usize,
}

impl LiveDub {
    #[must_use]
    pub fn new(lang: impl Into<String>, voice: Box<dyn Synthesizer>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            lang: lang.into(),
            voice: Arc::new(Mutex::new(voice)),
            tx,
            rx,
            in_flight: Arc::new(AtomicUsize::new(0)),
            speaks_until: None,
            skipped: 0,
        }
    }

    #[must_use]
    pub fn language(&self) -> &str {
        &self.lang
    }

    /// How far behind the dub is, in seconds of speech still to play.
    #[must_use]
    pub fn backlog_s(&self) -> f64 {
        self.backlog_at(Instant::now())
    }

    fn backlog_at(&self, now: Instant) -> f64 {
        self.speaks_until.map_or(0.0, |until| {
            until.saturating_duration_since(now).as_secs_f64()
        })
    }

    /// Lines the dub could not keep up with, clearing the count.
    ///
    /// Reported rather than logged, for the reason `live.rs` gives about dropped subtitles: a
    /// listener with gaps should be told the machine could not keep up, not left to think nobody
    /// spoke.
    pub fn take_skipped(&mut self) -> usize {
        std::mem::take(&mut self.skipped)
    }

    /// Feed the pipeline's events in; get audio out.
    ///
    /// Only translations into this dub's language are spoken. A meeting with two subtitle languages
    /// produces two `Translation` events per line and the listener chose one of them.
    pub fn offer(&mut self, events: &[Event]) -> Vec<Chunk> {
        self.offer_at(events, Instant::now())
    }

    /// The same, with the clock supplied — so the pacing has tests that do not sleep.
    pub fn offer_at(&mut self, events: &[Event], now: Instant) -> Vec<Chunk> {
        for event in events {
            let Event::Translation { seq, lang, text } = event else {
                continue;
            };
            if lang != &self.lang || text.trim().is_empty() {
                continue;
            }
            if self.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
                // Nothing is gained by queueing here. The voice is one session behind one lock, so
                // a queued line would wait exactly as long and arrive exactly as late — and by the
                // time it was said the backlog rule below would have skipped it anyway.
                self.skipped += 1;
                continue;
            }
            match pace(self.backlog_at(now)) {
                Pace::Skip => self.skipped += 1,
                Pace::Speak(speed) => self.say(*seq, text, speed),
            }
        }
        self.collect(now)
    }

    /// Everything finished since the last call, and the backlog updated by it.
    fn collect(&mut self, now: Instant) -> Vec<Chunk> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.rx.try_recv() {
            // Back to back from whichever is later: now, if the dub has run dry, or the end of what
            // is already playing. Taking `now` alone would forget everything still in the listener's
            // ears and let the controller think it was never behind.
            let from = self
                .speaks_until
                .filter(|until| *until > now)
                .unwrap_or(now);
            self.speaks_until = Some(from + Duration::from_secs_f64(chunk.duration_s()));
            out.push(chunk);
        }
        out
    }

    fn say(&self, seq: u64, text: &str, speed: f64) {
        let voice = self.voice.clone();
        let lang = self.lang.clone();
        let text = text.to_string();
        let tx = self.tx.clone();
        let in_flight = self.in_flight.clone();

        in_flight.fetch_add(1, Ordering::Relaxed);
        // `spawn_blocking`, because synthesis is unbroken CPU. On the async runtime it would stall
        // every other request the daemon is serving, including the socket carrying the transcript
        // of the meeting being dubbed.
        tokio::task::spawn_blocking(move || {
            let spoken = {
                let mut voice = voice.lock();
                voice.say_at(&text, &summo_tts::Voice::default(), speed as f32)
            };
            in_flight.fetch_sub(1, Ordering::Relaxed);
            match spoken {
                Ok(speech) if !speech.samples.is_empty() => {
                    let _ = tx.send(Chunk {
                        seq,
                        lang,
                        rate: speech.rate,
                        samples: speech.samples,
                    });
                }
                // Silence is not worth a frame, and the client would have to special-case it.
                Ok(_) => {}
                // One line costs its own audio, not the dub and not the recording. Logged rather
                // than raised: a voice that cannot say one line will usually say the next, and
                // stopping the whole feature over it is the larger failure.
                Err(e) => tracing::warn!(error = %e, seq, "a line could not be spoken"),
            }
        });
    }
}

impl std::fmt::Debug for LiveDub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDub")
            .field("lang", &self.lang)
            .field("backlog_s", &self.backlog_s())
            .field("skipped", &self.skipped)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_tts::{Speech, Voice};

    /// A voice that returns exactly as much audio as it was asked for, so a test can put a known
    /// duration into the controller.
    struct Metronome {
        /// Seconds of audio per call, before speed.
        seconds: f64,
    }

    impl Synthesizer for Metronome {
        fn rate(&self) -> u32 {
            16_000
        }
        fn say_at(
            &mut self,
            _text: &str,
            _voice: &Voice,
            speed: f32,
        ) -> summo_core::Result<Speech> {
            let seconds = self.seconds / f64::from(speed.max(0.1));
            Ok(Speech {
                samples: vec![0.1; (16_000.0 * seconds) as usize],
                rate: 16_000,
            })
        }
    }

    fn translation(seq: u64, lang: &str, text: &str) -> Event {
        Event::Translation {
            seq,
            lang: lang.to_string(),
            text: text.to_string(),
        }
    }

    fn dub(seconds: f64) -> LiveDub {
        LiveDub::new("en", Box::new(Metronome { seconds }))
    }

    /// Nothing in hand: say it at the pace the voice was trained at.
    #[test]
    fn a_dub_that_is_not_behind_does_not_hurry() {
        assert_eq!(pace(0.0), Pace::Speak(1.0));
        assert_eq!(pace(EASY_S), Pace::Speak(1.0));
    }

    /// And it rises smoothly rather than in steps: a listener notices a speed *change* far more
    /// than a speed.
    #[test]
    fn falling_behind_speeds_the_voice_up_gradually() {
        let Pace::Speak(halfway) = pace((EASY_S + DROP_S) / 2.0) else {
            panic!("halfway between easy and dropping should still be spoken");
        };
        assert!(
            halfway > 1.0 && halfway < MAX_SPEED,
            "expected a speed between the two ends, got {halfway}"
        );

        let Pace::Speak(nearly) = pace(DROP_S - 0.01) else {
            panic!("just short of the drop should still be spoken");
        };
        assert!(nearly > halfway, "the curve went the wrong way");
        assert!(nearly <= MAX_SPEED, "past the ceiling: {nearly}");
    }

    /// Past the point where hurrying stops being listenable, the line is not said at all.
    #[test]
    fn a_dub_too_far_behind_skips_rather_than_falls_further() {
        assert_eq!(pace(DROP_S), Pace::Skip);
        assert_eq!(pace(DROP_S + 10.0), Pace::Skip);
    }

    /// A speed is never invented from a number that is not one.
    #[test]
    fn a_backlog_that_is_not_a_number_is_treated_as_none() {
        assert_eq!(pace(f64::NAN), Pace::Speak(1.0));
        assert_eq!(pace(-5.0), Pace::Speak(1.0));
    }

    /// The listener chose one language. The other one's subtitles are not theirs to hear.
    #[tokio::test]
    async fn only_the_chosen_language_is_spoken() {
        let mut dub = dub(1.0);
        dub.offer(&[translation(1, "ja", "こんにちは")]);
        tokio::task::yield_now().await;
        assert!(
            dub.offer(&[]).is_empty(),
            "it spoke somebody else's subtitle"
        );
    }

    /// The ordinary path, end to end through the blocking thread.
    #[tokio::test]
    async fn a_translation_in_the_chosen_language_comes_back_as_audio() {
        let mut dub = dub(1.0);
        assert!(dub.offer(&[translation(7, "en", "hello there")]).is_empty());

        // Synthesis happens on a blocking thread, so the chunk arrives on a later call — which is
        // the whole reason `offer` returns what finished rather than what it started.
        let chunk = loop {
            if let Some(chunk) = dub.offer(&[]).into_iter().next() {
                break chunk;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(chunk.seq, 7);
        assert_eq!(chunk.lang, "en");
        assert!((chunk.duration_s() - 1.0).abs() < 0.01);
    }

    /// The backlog is what has been handed over and not yet finished playing, and it accumulates
    /// across chunks rather than being forgotten between them.
    #[tokio::test]
    async fn audio_already_sent_counts_against_the_next_line() {
        let mut dub = dub(2.0);
        let start = Instant::now();

        dub.offer_at(&[translation(1, "en", "one")], start);
        let mut sent = Vec::new();
        while sent.is_empty() {
            sent = dub.offer_at(&[], start);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!((dub.backlog_at(start) - 2.0).abs() < 0.01);

        // A second line, still at `start`, stacks on the end of the first rather than replacing it.
        dub.offer_at(&[translation(2, "en", "two")], start);
        let mut second = Vec::new();
        while second.is_empty() {
            second = dub.offer_at(&[], start);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            dub.backlog_at(start) > 3.0,
            "the second line did not stack: {}",
            dub.backlog_at(start)
        );
    }

    /// A gap in the conversation must clear the backlog. Measuring from `now` rather than from the
    /// end of the last chunk is what keeps a dub that has caught up from hurrying forever.
    #[tokio::test]
    async fn a_pause_lets_the_dub_catch_up() {
        let mut dub = dub(1.0);
        let start = Instant::now();
        dub.offer_at(&[translation(1, "en", "one")], start);
        while dub.offer_at(&[], start).is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(dub.backlog_at(start) > 0.5);
        assert_eq!(dub.backlog_at(start + Duration::from_secs(30)), 0.0);
    }

    /// Skipped lines are counted, because a gap nobody is told about reads as "nothing was said".
    #[test]
    fn lines_that_were_not_spoken_are_counted() {
        let mut dub = dub(1.0);
        dub.speaks_until = Some(Instant::now() + Duration::from_secs_f64(DROP_S + 1.0));
        dub.offer(&[translation(1, "en", "too far behind")]);
        assert_eq!(dub.take_skipped(), 1);
        assert_eq!(dub.take_skipped(), 0, "the count was not cleared");
    }

    /// Both halves of the wire format, against each other.
    #[test]
    fn a_frame_survives_the_round_trip() {
        let chunk = Chunk {
            seq: 4_294_967_296,
            lang: "vi".into(),
            rate: 22_050,
            samples: vec![-1.0, 0.0, 0.5, 1.0],
        };
        assert_eq!(Chunk::decode(&chunk.encode()), Some(chunk));
    }

    /// Anything that is not a dub frame is not decoded into one. The socket carries other bytes.
    #[test]
    fn a_frame_that_is_not_a_dub_is_refused() {
        assert_eq!(Chunk::decode(&[]), None);
        assert_eq!(Chunk::decode(&[0x00, 1, b'v']), None, "wrong tag");
        assert_eq!(Chunk::decode(&[TAG_DUB, 2, b'v']), None, "truncated");
        // A body that is not a whole number of samples is a corrupt frame, not a short one.
        assert_eq!(
            Chunk::decode(&[TAG_DUB, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3]),
            None
        );
    }
}
