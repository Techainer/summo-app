//! Stages that need no model.
//!
//! The interesting processors — voice detection, recognition, translation — live where their
//! dependencies do, because putting them here would drag ONNX Runtime into a crate whose whole
//! value is being testable without it. What is here is the plumbing every pipeline needs and
//! nothing else: reframing, routing, tapping, counting.

use std::collections::HashMap;

use summo_core::{Event, Result, segment::Lane};

use crate::{Frame, Processor};

/// Re-cut audio into blocks of an exact width.
///
/// Voice detectors want a fixed frame — Silero's recurrent state is defined per frame, and feeding
/// it a different width silently corrupts that state rather than failing. Callers send whatever
/// their capture device produced, so something has to sit between, and it has to buffer across
/// pushes: a 100 ms block does not divide into 30 ms frames.
///
/// The remainder is held, not emitted. On [`Frame::Flush`] it goes out zero-padded, because the
/// last 12 ms of a meeting is a word somebody said.
pub struct Reframe {
    width: usize,
    /// Per lane: two lanes interleaved into one buffer would produce frames of mixed audio.
    buffers: HashMap<Lane, Vec<f32>>,
    rate: u32,
}

impl Reframe {
    #[must_use]
    pub fn new(width: usize, rate: u32) -> Self {
        Self {
            width: width.max(1),
            buffers: HashMap::new(),
            rate,
        }
    }

    fn drain(&mut self, lane: Lane, pad: bool) -> Vec<Frame> {
        let Some(buffer) = self.buffers.get_mut(&lane) else {
            return Vec::new();
        };
        let mut out = Vec::new();

        while buffer.len() >= self.width {
            let frame: Vec<f32> = buffer.drain(..self.width).collect();
            out.push(Frame::audio(lane, frame, self.rate));
        }

        if pad && !buffer.is_empty() {
            let mut last = std::mem::take(buffer);
            last.resize(self.width, 0.0);
            out.push(Frame::audio(lane, last, self.rate));
        }
        out
    }
}

impl Processor for Reframe {
    fn name(&self) -> &'static str {
        "reframe"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match frame {
            Frame::Audio(audio) => {
                self.buffers
                    .entry(audio.lane)
                    .or_default()
                    .extend_from_slice(&audio.samples);
                Ok(self.drain(audio.lane, false))
            }
            Frame::Flush | Frame::End => {
                let lanes: Vec<Lane> = self.buffers.keys().copied().collect();
                let mut out = Vec::new();
                for lane in lanes {
                    out.extend(self.drain(lane, true));
                }
                out.push(frame);
                Ok(out)
            }
            other => Ok(vec![other]),
        }
    }

    fn reset(&mut self) {
        self.buffers.clear();
    }
}

/// Drop everything for lanes this pipeline does not handle.
///
/// A session that opened only the microphone should not spend a decode on system audio a client
/// sent by mistake. Control frames pass regardless — they belong to the pipeline, not to a lane.
pub struct OnlyLanes {
    lanes: Vec<Lane>,
}

impl OnlyLanes {
    #[must_use]
    pub fn new(lanes: impl IntoIterator<Item = Lane>) -> Self {
        Self {
            lanes: lanes.into_iter().collect(),
        }
    }
}

impl Processor for OnlyLanes {
    fn name(&self) -> &'static str {
        "only-lanes"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match frame.lane() {
            Some(lane) if !self.lanes.contains(&lane) => Ok(vec![]),
            _ => Ok(vec![frame]),
        }
    }
}

/// Watch frames go past without changing them.
///
/// For the things that have to happen alongside the stream rather than in it: archiving audio to
/// disk, updating a progress counter, writing the transcript file. A tap that modified frames would
/// be a processor; this one is for effects.
pub struct Tap<F: FnMut(&Frame) + Send> {
    label: &'static str,
    on: F,
}

impl<F: FnMut(&Frame) + Send> Tap<F> {
    #[must_use]
    pub fn new(label: &'static str, on: F) -> Self {
        Self { label, on }
    }
}

impl<F: FnMut(&Frame) + Send> Processor for Tap<F> {
    fn name(&self) -> &'static str {
        self.label
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        (self.on)(&frame);
        Ok(vec![frame])
    }
}

/// How much audio and how many utterances went through.
///
/// Kept as a processor rather than computed at the edges because the edges disagree: the socket
/// sees what a client sent, the recorder sees what survived the gate, and the status endpoint wants
/// the second.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Counts {
    pub seconds: f64,
    pub finals: u64,
}

pub struct Meter {
    counts: Counts,
}

impl Meter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            counts: Counts::default(),
        }
    }

    #[must_use]
    pub fn counts(&self) -> Counts {
        self.counts
    }
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for Meter {
    fn name(&self) -> &'static str {
        "meter"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match &frame {
            Frame::Audio(audio) => self.counts.seconds += audio.duration_s(),
            Frame::Event(Event::Final(_)) => self.counts.finals += 1,
            _ => {}
        }
        Ok(vec![frame])
    }

    fn reset(&mut self) {
        self.counts = Counts::default();
    }
}

/// Keep only the frames the interface needs to see.
///
/// The last stage of a pipeline whose output goes to a socket. Audio and voice probabilities are
/// internal: forwarding them would mean the daemon streaming raw PCM back to the app that sent it.
pub struct EventsOnly;

impl Processor for EventsOnly {
    fn name(&self) -> &'static str {
        "events-only"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match frame {
            Frame::Event(_) => Ok(vec![frame]),
            other if other.is_control() => Ok(vec![other]),
            _ => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pipeline;
    use summo_core::segment::Segment;

    fn audio(lane: Lane, n: usize) -> Frame {
        Frame::audio(lane, vec![0.5; n], 16_000)
    }

    fn samples_of(frames: &[Frame]) -> Vec<usize> {
        frames
            .iter()
            .filter_map(|f| match f {
                Frame::Audio(a) => Some(a.samples.len()),
                _ => None,
            })
            .collect()
    }

    /// Silero's recurrent state is defined per frame width; a different width corrupts it silently
    /// rather than failing, which is the worst way for this to go wrong.
    #[test]
    fn reframe_cuts_to_an_exact_width() {
        let mut p = Reframe::new(160, 16_000);
        let out = p.push(audio(Lane::Mic, 500)).unwrap();
        assert_eq!(samples_of(&out), [160, 160, 160], "and 20 held back");
    }

    #[test]
    fn reframe_buffers_across_pushes() {
        let mut p = Reframe::new(160, 16_000);
        assert!(samples_of(&p.push(audio(Lane::Mic, 100)).unwrap()).is_empty());
        assert_eq!(samples_of(&p.push(audio(Lane::Mic, 100)).unwrap()), [160]);
    }

    /// Two lanes sharing a buffer would produce frames of mixed audio — one speaker's voice
    /// interleaved with another's, at frame boundaries nobody could see.
    #[test]
    fn reframe_keeps_lanes_apart() {
        let mut p = Reframe::new(160, 16_000);
        p.push(audio(Lane::Mic, 100)).unwrap();
        assert!(
            samples_of(&p.push(audio(Lane::System, 100)).unwrap()).is_empty(),
            "the system lane has 100 of its own, not 200 shared"
        );
    }

    /// The last 12 ms of a meeting is a word somebody said.
    #[test]
    fn reframe_pads_the_remainder_on_flush() {
        let mut p = Reframe::new(160, 16_000);
        p.push(audio(Lane::Mic, 100)).unwrap();
        let out = p.push(Frame::Flush).unwrap();
        assert_eq!(samples_of(&out), [160], "zero-padded to a whole frame");
        assert!(out.last().unwrap().is_control(), "and the flush still travels");
    }

    #[test]
    fn reframe_emits_nothing_extra_when_it_divides_evenly() {
        let mut p = Reframe::new(160, 16_000);
        p.push(audio(Lane::Mic, 320)).unwrap();
        assert!(samples_of(&p.push(Frame::Flush).unwrap()).is_empty());
    }

    #[test]
    fn reframe_forgets_its_buffer_on_reset() {
        let mut p = Reframe::new(160, 16_000);
        p.push(audio(Lane::Mic, 100)).unwrap();
        p.reset();
        assert!(samples_of(&p.push(Frame::Flush).unwrap()).is_empty());
    }

    #[test]
    fn a_zero_width_is_clamped_rather_than_looping_forever() {
        let mut p = Reframe::new(0, 16_000);
        assert_eq!(samples_of(&p.push(audio(Lane::Mic, 3)).unwrap()).len(), 3);
    }

    #[test]
    fn only_lanes_drops_what_the_session_did_not_open() {
        let mut p = OnlyLanes::new([Lane::Mic]);
        assert!(!p.push(audio(Lane::Mic, 10)).unwrap().is_empty());
        assert!(p.push(audio(Lane::System, 10)).unwrap().is_empty());
    }

    /// Control frames belong to the pipeline, not to a lane, so a lane filter must let them past.
    #[test]
    fn only_lanes_never_drops_a_control_frame() {
        let mut p = OnlyLanes::new([Lane::Mic]);
        for control in [Frame::Start, Frame::Flush, Frame::End] {
            assert_eq!(p.push(control.clone()).unwrap(), vec![control]);
        }
    }

    #[test]
    fn a_tap_sees_frames_without_changing_them() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // `Arc`, not `Rc`: a processor is `Send`, because a pipeline is built on one thread and
        // driven on another — the socket task hands it to the decode thread.
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        let mut p = Tap::new("probe", move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });

        let out = p.push(audio(Lane::Mic, 10)).unwrap();
        assert_eq!(out.len(), 1, "a tap observes, it does not consume");
        assert_eq!(seen.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_meter_counts_audio_and_finals() {
        let mut p = Meter::new();
        p.push(Frame::audio(Lane::Mic, vec![0.0; 16_000], 16_000)).unwrap();
        p.push(Frame::Event(Event::Final(Segment::new(
            1,
            Lane::Mic,
            "hi",
            0.0,
            1.0,
        ))))
        .unwrap();
        p.push(Frame::Event(Event::info("not a final"))).unwrap();

        assert_eq!(p.counts(), Counts { seconds: 1.0, finals: 1 });
        p.reset();
        assert_eq!(p.counts(), Counts::default());
    }

    /// Forwarding audio here would mean the daemon streaming raw PCM back to the app that sent it.
    #[test]
    fn events_only_keeps_events_and_control_frames() {
        let mut p = EventsOnly;
        assert!(p.push(audio(Lane::Mic, 10)).unwrap().is_empty());
        assert!(
            p.push(Frame::Voice {
                lane: Lane::Mic,
                probability: 0.9
            })
            .unwrap()
            .is_empty()
        );
        assert_eq!(p.push(Frame::End).unwrap(), vec![Frame::End]);
        assert_eq!(p.push(Frame::Event(Event::info("x"))).unwrap().len(), 1);
    }

    /// The whole point: a chain assembled from parts, behaving as one thing.
    #[test]
    fn a_chain_reframes_filters_and_counts_together() {
        let mut pipeline = Pipeline::new()
            .then(OnlyLanes::new([Lane::Mic]))
            .then(Reframe::new(160, 16_000))
            .then(Meter::new());

        assert_eq!(pipeline.describe(), "only-lanes → reframe → meter");

        // System audio is dropped before it costs anything.
        assert!(pipeline.push(audio(Lane::System, 1_600)).unwrap().is_empty());

        let out = pipeline.push(audio(Lane::Mic, 500)).unwrap();
        assert_eq!(samples_of(&out), [160, 160, 160]);

        // And the tail still comes out, padded, when the stream ends.
        let tail = pipeline.push(Frame::End).unwrap();
        assert_eq!(samples_of(&tail), [160]);
        assert!(tail.iter().any(Frame::is_end));
    }

    /// Inserting a stage is a local change — that is the property the pipeline is for.
    #[test]
    fn a_stage_can_be_inserted_without_touching_the_others() {
        let mut without = Pipeline::new().then(Reframe::new(160, 16_000));
        let mut with = Pipeline::new()
            .then(OnlyLanes::new([Lane::Mic, Lane::System]))
            .then(Reframe::new(160, 16_000));

        let a = without.push(audio(Lane::Mic, 480)).unwrap();
        let b = with.push(audio(Lane::Mic, 480)).unwrap();
        assert_eq!(samples_of(&a), samples_of(&b));
    }
}
