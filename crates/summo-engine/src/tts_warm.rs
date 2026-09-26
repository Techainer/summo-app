//! One voice, kept loaded, so the first dubbed line does not wait for it.
//!
//! The same shape as [`crate::warm`], for the same measured reason and with the same trade-offs
//! already argued there. What differs is only the number and what fills the slot.
//!
//! Constructing a VITS voice costs **1.7 to 1.9 seconds** — measured on this machine with
//! `vits-piper-en_US-amy-medium` through `summo-tts`'s `speak` example, which prints it. Synthesis
//! itself is nothing: a real-time factor of **0.065**, so a three-second line is spoken in about
//! two hundred milliseconds, and the cost is near enough linear in the length of the text.
//!
//! That ratio is the whole argument. Loading is roughly nine times the cost of saying a sentence,
//! it is paid once, and paying it when the first line arrives puts it squarely in the path of the
//! thing a listener is waiting for. Paid when they *choose the language* instead, it lands in the
//! seconds before anybody has said anything.
//!
//! The offline dub does not need this — it loads once and then speaks a whole meeting, so the load
//! is a rounding error against the work. Live dubbing is the opposite: one short line at a time,
//! the first of them the one that decides whether the feature feels instant.
//!
//! Deliberately one slot, given away rather than lent, dropped after [`IDLE`] — see `warm.rs` for
//! why each of those is right. A voice is smaller than a decoder but not free, and a slot that is
//! never emptied is the bug that module already had once.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use summo_tts::Synthesizer;

/// How long a warm voice is worth its memory.
///
/// The same twenty minutes as the decoder's, and for the same reason: longer than the pause between
/// two meetings in a morning, far shorter than a working day. Two different answers to the same
/// question would be two numbers to keep in step with nothing keeping them.
pub const IDLE: Duration = crate::warm::IDLE;

/// What a warm voice was built for. A slot holding the wrong voice is a miss.
///
/// The directory rather than the model id: a voice is resolved to a path before it is loaded, two
/// registry entries can resolve to the same directory, and the path is what the loader was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub dir: std::path::PathBuf,
    pub threads: usize,
}

impl Key {
    #[must_use]
    pub fn new(dir: impl Into<std::path::PathBuf>, threads: usize) -> Self {
        Self {
            dir: dir.into(),
            threads,
        }
    }
}

/// What the slot holds, and when it was put there.
type Held = (Key, Box<dyn Synthesizer>, Instant);

/// The slot.
#[derive(Default)]
pub struct WarmVoice {
    slot: Mutex<Option<Held>>,
}

impl WarmVoice {
    /// Take the voice if it is the one being asked for.
    ///
    /// A miss is not a fallback to something similar. A VITS voice handed a language it has no
    /// phoneme table for does not fail — it runs the text through the table it has and says the
    /// result, confidently, over somebody's meeting. Returning the wrong voice quickly is worse
    /// than loading the right one slowly.
    pub fn take(&self, key: &Key) -> Option<Box<dyn Synthesizer>> {
        let mut slot = self.slot.lock().ok()?;
        match slot.as_ref() {
            Some((held, _, _)) if held == key => slot.take().map(|(_, voice, _)| voice),
            _ => None,
        }
    }

    /// Put a freshly loaded voice in the slot, replacing whatever was there.
    pub fn put(&self, key: Key, voice: Box<dyn Synthesizer>) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some((key, voice, Instant::now()));
        }
    }

    /// Drop the voice if nothing has wanted it for [`IDLE`].
    ///
    /// Called on the same timer as the decoder's. Returns what it dropped so the caller can say so
    /// in a log, rather than leaving a change in resident memory unexplained.
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
    /// Called when the voice it holds is removed: a warm voice pointing at deleted blobs is a crash
    /// waiting for the next line.
    pub fn clear(&self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = None;
        }
    }
}

impl std::fmt::Debug for WarmVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarmVoice")
            .field("ready", &self.ready())
            .finish()
    }
}

/// Load a voice for a directory, ready to be put in the slot.
///
/// Goes through the same loader a live dub would use, so a warm voice is built by exactly the rules
/// that will ask for it. Built differently it would miss on every take, and the slot would be
/// memory held for no benefit — the failure `warm.rs` names in its own `build`.
pub fn build(
    dir: &std::path::Path,
    threads: usize,
) -> summo_core::Result<(Key, Box<dyn Synthesizer>)> {
    let voice = summo_tts::vits::Vits::load(dir, threads)?;
    Ok((Key::new(dir, threads), Box::new(voice)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_tts::{Speech, Voice};

    struct Fake;

    impl Synthesizer for Fake {
        fn rate(&self) -> u32 {
            22_050
        }
        fn say(&mut self, _text: &str, _voice: &Voice) -> summo_core::Result<Speech> {
            Ok(Speech {
                samples: Vec::new(),
                rate: 22_050,
            })
        }
    }

    fn key() -> Key {
        Key::new("/voices/vits-vi-vais1000", 8)
    }

    /// The point of the slot: the second asker does not pay the 1.7 seconds the first one did.
    #[test]
    fn a_voice_already_loaded_is_handed_over_rather_than_built_again() {
        let warm = WarmVoice::default();
        warm.put(key(), Box::new(Fake));
        assert!(warm.take(&key()).is_some());
        // Given away, not lent. Whoever took it owns it, so there is no question about what
        // happens to a borrowed voice when a recording is killed.
        assert!(warm.take(&key()).is_none(), "the slot lent it instead");
    }

    /// A voice for another language would speak the line in that language's phoneme table. Loading
    /// the right one slowly beats answering with the wrong one instantly.
    #[test]
    fn a_slot_holding_another_voice_is_a_miss_not_a_substitute() {
        let warm = WarmVoice::default();
        warm.put(Key::new("/voices/vits-en-ljspeech", 8), Box::new(Fake));
        assert!(warm.take(&key()).is_none());
        assert_eq!(
            warm.ready(),
            Some(Key::new("/voices/vits-en-ljspeech", 8)),
            "a miss must not consume the slot"
        );
    }

    /// Threads are part of what was built, not a detail beside it — a voice loaded for four threads
    /// is a different ONNX session from one loaded for eight.
    #[test]
    fn the_thread_count_is_part_of_what_was_built() {
        let warm = WarmVoice::default();
        warm.put(key(), Box::new(Fake));
        assert!(
            warm.take(&Key::new("/voices/vits-vi-vais1000", 4))
                .is_none()
        );
    }

    /// The bug `warm.rs` had: a slot filled once and emptied only by deleting the model in it.
    #[test]
    fn a_voice_nobody_came_back_for_is_given_back() {
        let warm = WarmVoice::default();
        warm.put(key(), Box::new(Fake));

        // Just short of the deadline: somebody between two meetings still gets the fast start.
        assert_eq!(
            warm.evict_idle(Instant::now() + IDLE - Duration::from_secs(1)),
            None
        );
        assert_eq!(warm.ready(), Some(key()));

        assert_eq!(warm.evict_idle(Instant::now() + IDLE), Some(key()));
        assert_eq!(warm.ready(), None, "the memory was not released");
    }

    /// Removing the voice must empty the slot: a warm voice pointing at deleted blobs is a crash.
    #[test]
    fn clearing_releases_it() {
        let warm = WarmVoice::default();
        warm.put(key(), Box::new(Fake));
        warm.clear();
        assert_eq!(warm.ready(), None);
    }
}
