//! One speech model, kept loaded, so pressing record does not wait for it.
//!
//! Constructing a decoder costs about **3.4 seconds** on this machine — measured on the released
//! build with `gipformer-65m`, from `session_start` to the daemon answering `session started`. It is
//! not disk: the second construction in the same process takes the same time as the first, because
//! what it spends is ONNX Runtime building the session, not reading 70 MB.
//!
//! And it was paid *per meeting*. Every recording rebuilt everything, so the three and a half
//! seconds after pressing record — with no indication anything was happening — were a fixed tax on
//! the one action the whole product is about.
//!
//! So one decoder is kept ready. Deliberately one, not a cache of many:
//!
//! * **One model.** A second warm model would double resident memory for the case where somebody
//!   switches languages between meetings, which is rarer than recording twice in the same one.
//! * **One lane.** A two-lane session — microphone plus system audio — still constructs its second
//!   decoder, because a decoder holds mutable inference state and two lanes cannot share one.
//!   Half of a rare case is instant; all of the common case is.
//! * **Given away, not lent.** A session takes the decoder out of the slot and owns it. The slot is
//!   refilled in the background afterwards, which keeps the code free of any question about what
//!   happens to a borrowed decoder when a recording is killed.
//!
//! Memory is the cost: `gipformer-65m` idles at about 150 MB. That is a real amount to hold for an
//! app that is not recording, and it is why this is filled on demand — after an install, after a
//! session, when the interface asks — rather than at startup regardless of whether anyone intends
//! to record.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use summo_asr::Decoder;

/// How long a warm decoder is worth its memory.
///
/// The slot was filled and never emptied: the only thing that cleared it was *deleting* the model
/// it held. So a machine that recorded once at nine in the morning held about 150 MB of decoder
/// until the app was quit — for a second recording that, past a certain gap, is not coming. Twenty
/// minutes is longer than the pause between two meetings in a morning and far shorter than a
/// working day, which is the distinction that matters: the tax this module exists to avoid is paid
/// again only by somebody who has already stopped.
const IDLE: Duration = Duration::from_secs(20 * 60);

/// What a warm decoder was built for. A slot for the wrong language is a miss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub model: String,
    /// `None` means the model's own detection, which is a different decoder from a named language.
    pub language: Option<String>,
    pub threads: usize,
}

impl Key {
    #[must_use]
    pub fn new(model: impl Into<String>, language: Option<String>, threads: usize) -> Self {
        Self {
            model: model.into(),
            language: language
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty()),
            threads,
        }
    }
}

/// What the slot holds: what it was built for, the decoder itself, and when it was put there.
///
/// The instant is what makes [`Warm::evict_idle`] possible — without it the slot had no idea how
/// long its 150 MB had been unwanted.
type Held = (Key, Box<dyn Decoder>, Instant);

/// The slot.
#[derive(Default)]
pub struct Warm {
    slot: Mutex<Option<Held>>,
}

impl Warm {
    /// Take the decoder if it is the one being asked for.
    ///
    /// A miss is not an error and not a fallback to something similar: a decoder built for another
    /// language would transcribe the meeting in that language, which is the failure this whole
    /// area of the app exists to prevent.
    pub fn take(&self, key: &Key) -> Option<Box<dyn Decoder>> {
        let mut slot = self.slot.lock().ok()?;
        match slot.as_ref() {
            Some((held, _, _)) if held == key => slot.take().map(|(_, decoder, _)| decoder),
            _ => None,
        }
    }

    /// Drop the decoder if nothing has wanted it for [`IDLE`].
    ///
    /// Called on a timer. Returns what it dropped, so the caller can say so in a log rather than
    /// leaving a 150 MB change in resident memory unexplained.
    pub fn evict_idle(&self, now: Instant) -> Option<Key> {
        let mut slot = self.slot.lock().ok()?;
        let stale = slot
            .as_ref()
            .is_some_and(|(_, _, since)| now.duration_since(*since) >= IDLE);
        if !stale {
            return None;
        }
        slot.take().map(|(key, _, _)| key)
    }

    /// Put a freshly built decoder in the slot, replacing whatever was there.
    ///
    /// Replacing rather than keeping both: the newest request is the best guess at what the next
    /// recording will want, and holding two models is the memory decision this module exists to
    /// avoid.
    pub fn put(&self, key: Key, decoder: Box<dyn Decoder>) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some((key, decoder, Instant::now()));
        }
    }

    /// What is ready, for the interface to say so.
    #[must_use]
    pub fn ready(&self) -> Option<Key> {
        self.slot
            .lock()
            .ok()?
            .as_ref()
            .map(|(key, _, _)| key.clone())
    }

    /// Drop whatever is held, freeing its memory.
    ///
    /// Called when the model it holds is removed: a warm decoder pointing at deleted blobs is a
    /// crash waiting for the next recording.
    pub fn clear(&self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = None;
        }
    }
}

impl std::fmt::Debug for Warm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The decoder itself has no useful `Debug`, and printing a model's weights is not what
        // anybody wants from `{:?}` on the engine state.
        f.debug_struct("Warm")
            .field("ready", &self.ready())
            .finish()
    }
}

/// Build a decoder for a spec, ready to be put in the slot.
///
/// Uses the session's own loader, so a warm decoder is built by exactly the rules a session would
/// use — same model resolution, same language, same thread count. Built differently, it would miss
/// on every take and the slot would be a memory leak with no benefit.
pub fn build(
    spec: &crate::protocol::SessionSpec,
    store: &summo_models::ModelStore,
    hw: &summo_models::HwProfile,
) -> summo_core::Result<(Key, Box<dyn Decoder>)> {
    let threads = hw.recommended_threads();
    let decoder =
        crate::runner::load_decoder(&spec.live_model, spec.language.as_deref(), store, threads)?;
    Ok((
        Key::new(&spec.live_model, spec.language.clone(), threads),
        decoder,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_asr::Transcript;

    struct Fake(&'static str);

    impl Decoder for Fake {
        fn decode(&mut self, _pcm: &[f32]) -> summo_core::Result<Transcript> {
            Ok(Transcript::default())
        }
        fn name(&self) -> &str {
            self.0
        }
    }

    /// The slot used to be filled and never emptied — the only thing that cleared it was deleting
    /// the model it held — so a machine that recorded once in the morning carried the decoder's
    /// memory until the app was quit.
    #[test]
    fn a_decoder_nobody_came_back_for_is_given_back() {
        let warm = Warm::default();
        let key = Key::new("gipformer-65m", Some("vi".into()), 8);
        warm.put(key.clone(), Box::new(Fake("gipformer-65m")));

        // Just short of the deadline: somebody between two meetings still gets the fast start.
        assert_eq!(
            warm.evict_idle(Instant::now() + IDLE - Duration::from_secs(1)),
            None
        );
        assert_eq!(warm.ready(), Some(key.clone()));

        assert_eq!(warm.evict_idle(Instant::now() + IDLE), Some(key));
        assert_eq!(warm.ready(), None, "the memory was not released");
        // And an empty slot is not something to keep reporting.
        assert_eq!(warm.evict_idle(Instant::now() + IDLE * 10), None);
    }

    /// Putting one back is what a finished recording does, and it starts the clock again.
    #[test]
    fn refilling_the_slot_resets_the_idle_clock() {
        let warm = Warm::default();
        let key = Key::new("gipformer-65m", None, 8);
        warm.put(key.clone(), Box::new(Fake("a")));
        warm.put(key.clone(), Box::new(Fake("b")));
        assert_eq!(
            warm.evict_idle(Instant::now() + IDLE - Duration::from_secs(30)),
            None
        );
    }

    #[test]
    fn a_warm_decoder_is_handed_over_once() {
        let warm = Warm::default();
        let key = Key::new("gipformer-65m", Some("vi".into()), 8);
        warm.put(key.clone(), Box::new(Fake("gipformer-65m")));

        assert_eq!(warm.ready().as_ref(), Some(&key));
        assert!(warm.take(&key).is_some(), "the first taker gets it");
        assert!(warm.take(&key).is_none(), "and it is gone afterwards");
        assert!(warm.ready().is_none());
    }

    /// The failure this prevents: a meeting in Japanese decoded by a decoder built for Vietnamese,
    /// instantly and silently, because it happened to be the one already loaded.
    #[test]
    fn a_decoder_for_another_language_is_a_miss_not_a_substitute() {
        let warm = Warm::default();
        warm.put(
            Key::new("whisper-tiny", Some("vi".into()), 8),
            Box::new(Fake("whisper-tiny")),
        );

        assert!(
            warm.take(&Key::new("whisper-tiny", Some("ja".into()), 8))
                .is_none()
        );
        assert!(
            warm.take(&Key::new("gipformer-65m", Some("vi".into()), 8))
                .is_none()
        );
        // Detection is its own answer, not a wildcard that matches a named language.
        assert!(warm.take(&Key::new("whisper-tiny", None, 8)).is_none());
        assert!(
            warm.take(&Key::new("whisper-tiny", Some("vi".into()), 8))
                .is_some()
        );
    }

    /// `""` and `None` both mean "the model decides", and a slot filled by one must be found by the
    /// other — the interface sends an empty string, the daemon holds an `Option`.
    #[test]
    fn an_empty_language_is_the_same_request_as_none() {
        let warm = Warm::default();
        warm.put(
            Key::new("whisper-tiny", Some("  ".into()), 8),
            Box::new(Fake("w")),
        );
        assert!(warm.take(&Key::new("whisper-tiny", None, 8)).is_some());
    }

    #[test]
    fn clearing_frees_the_slot() {
        let warm = Warm::default();
        let key = Key::new("m", None, 4);
        warm.put(key.clone(), Box::new(Fake("m")));
        warm.clear();
        assert!(warm.take(&key).is_none());
    }
}
