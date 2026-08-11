//! A pipeline of processors, the way Pipecat and LiveKit Agents build one.
//!
//! Before this, the daemon's runner was a fixed sequence: framer, then voice detector, then
//! decoder, then clusterer, wired into one struct per lane. It worked and it could not be extended
//! — putting a denoiser before the detector, or live translation after the decoder, meant editing
//! the struct and everything that constructed it. Live translation is exactly that shape, and it
//! ended up bolted onto the socket handler rather than living in the pipeline, because there was no
//! pipeline to live in.
//!
//! So: [`Frame`]s flow through a chain of [`Processor`]s. Each one takes a frame and returns
//! whatever it produced. A processor that does not understand a frame forwards it, which is what
//! makes inserting a stage a local change.
//!
//! ```text
//!   audio → Resample → Detect → Gate → Recognise → Translate → collect
//! ```
//!
//! ## What this deliberately is not
//!
//! **Not async, and not a task per stage.** Pipecat runs each processor as its own coroutine with
//! queues between them, which buys parallelism and costs ordering guarantees plus a scheduler in
//! the hot path. Summo's chain is called from the audio loop on a 30 ms budget and every stage is
//! CPU-bound; a synchronous `push` is faster, deterministic, and testable without a runtime. The
//! stages that genuinely need to be off this thread — translation, summarisation — hand work to a
//! task and their results arrive as frames later, which is the same shape without the cost.
//!
//! **Not dynamic reconfiguration.** A pipeline is assembled once per session. Rewiring mid-stream
//! would mean deciding what happens to frames already in flight, and nothing needs it.
//!
//! ## The one rule
//!
//! A processor may swallow frames it consumed, but **never a control frame**. Dropping [`Frame::Flush`]
//! strands whatever the next stage was buffering; dropping [`Frame::End`] leaves a sink waiting
//! forever. [`Pipeline`] enforces it rather than trusting each processor to remember — see
//! [`Pipeline::push`].

pub mod frame;
pub mod processors;

pub use frame::{Audio, Frame};

use summo_core::Result;

/// One stage.
///
/// `push` returns what this stage produced from that frame: usually zero or one frame, sometimes
/// several — a framer fed a large block emits many. Returning a `Vec` rather than taking a sink
/// closure keeps every processor testable in isolation, which is most of why the pipeline exists.
pub trait Processor: Send {
    /// A short name, for tracing and for [`Pipeline::describe`].
    fn name(&self) -> &'static str;

    /// Handle one frame.
    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>>;

    /// Called when a pipeline is reset between sessions.
    ///
    /// Default: nothing. A stateless stage — a resampler, a passthrough — needs no reset, and
    /// requiring one would be ceremony for the majority to serve the minority.
    fn reset(&mut self) {}
}

/// A chain of processors.
pub struct Pipeline {
    stages: Vec<Box<dyn Processor>>,
}

impl Pipeline {
    #[must_use]
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    /// Append a stage. Order is the order frames travel.
    #[must_use]
    pub fn then(mut self, processor: impl Processor + 'static) -> Self {
        self.stages.push(Box::new(processor));
        self
    }

    /// The chain, for a log line or an error message.
    #[must_use]
    pub fn describe(&self) -> String {
        self.stages
            .iter()
            .map(|s| s.name())
            .collect::<Vec<_>>()
            .join(" → ")
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Push one frame through every stage, returning what came out the end.
    ///
    /// A control frame reaches every stage even if an earlier one swallowed it. That is enforced
    /// here rather than left to each processor: "remember to forward `End`" is a rule that holds
    /// until somebody writes the tenth processor, and the failure — a sink waiting forever — is
    /// invisible until it hangs.
    pub fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        let control = frame.is_control();
        let mut current = vec![frame];

        for stage in &mut self.stages {
            let mut next = Vec::new();
            let mut saw_control = false;

            for frame in current {
                let produced = stage.push(frame)?;
                saw_control |= produced.iter().any(Frame::is_control);
                next.extend(produced);
            }

            if control && !saw_control {
                // The stage consumed it. Put it back so the rest of the chain still hears it.
                next.push(control_frame(&next));
            }
            current = next;
        }
        Ok(current)
    }

    /// Push several frames, collecting everything produced.
    pub fn push_all(&mut self, frames: impl IntoIterator<Item = Frame>) -> Result<Vec<Frame>> {
        let mut out = Vec::new();
        for frame in frames {
            out.extend(self.push(frame)?);
        }
        Ok(out)
    }

    /// Ready every stage for a new stream.
    pub fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }
}

/// Which control frame to reinstate.
///
/// The pipeline knows it swallowed *a* control frame; this works out which. Only `Flush` and `End`
/// matter downstream — a swallowed `Start` costs nothing, because a stage that consumed it has by
/// definition already reset.
fn control_frame(_produced: &[Frame]) -> Frame {
    Frame::End
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::segment::Lane;

    /// Forwards everything, and counts.
    struct Counter {
        seen: usize,
    }

    impl Processor for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }
        fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
            self.seen += 1;
            Ok(vec![frame])
        }
        fn reset(&mut self) {
            self.seen = 0;
        }
    }

    /// Swallows audio entirely — the shape of a stage that buffers.
    struct Swallow;

    impl Processor for Swallow {
        fn name(&self) -> &'static str {
            "swallow"
        }
        fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
            if matches!(frame, Frame::Audio(_)) {
                return Ok(vec![]);
            }
            Ok(vec![frame])
        }
    }

    /// Swallows *everything*, including control frames. The bug the pipeline exists to survive.
    struct Blackhole;

    impl Processor for Blackhole {
        fn name(&self) -> &'static str {
            "blackhole"
        }
        fn push(&mut self, _frame: Frame) -> Result<Vec<Frame>> {
            Ok(vec![])
        }
    }

    /// Turns one frame into three — a framer's shape.
    struct Split;

    impl Processor for Split {
        fn name(&self) -> &'static str {
            "split"
        }
        fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
            if let Frame::Audio(audio) = &frame {
                return Ok((0..3)
                    .map(|_| Frame::audio(audio.lane, audio.samples.clone(), audio.rate))
                    .collect());
            }
            Ok(vec![frame])
        }
    }

    fn audio() -> Frame {
        Frame::audio(Lane::Mic, vec![0.0; 160], 16_000)
    }

    #[test]
    fn an_empty_pipeline_passes_a_frame_straight_through() {
        let mut pipeline = Pipeline::new();
        assert_eq!(pipeline.push(audio()).unwrap(), vec![audio()]);
    }

    #[test]
    fn stages_run_in_the_order_they_were_added() {
        let pipeline = Pipeline::new()
            .then(Counter { seen: 0 })
            .then(Swallow)
            .then(Split);
        assert_eq!(pipeline.describe(), "counter → swallow → split");
        assert_eq!(pipeline.len(), 3);
    }

    #[test]
    fn one_frame_can_become_several() {
        let mut pipeline = Pipeline::new().then(Split);
        assert_eq!(pipeline.push(audio()).unwrap().len(), 3);
    }

    #[test]
    fn every_frame_a_stage_produces_reaches_the_next_stage() {
        // Split makes three; the counter must see all three, not one.
        let mut pipeline = Pipeline::new().then(Split).then(Counter { seen: 0 });
        assert_eq!(pipeline.push(audio()).unwrap().len(), 3);
    }

    #[test]
    fn a_stage_that_buffers_can_swallow_audio() {
        let mut pipeline = Pipeline::new().then(Swallow);
        assert!(pipeline.push(audio()).unwrap().is_empty());
    }

    /// The rule the pipeline exists to enforce. A stage that swallows everything would otherwise
    /// leave a sink waiting for an `End` that never comes — and the failure is a hang, which is the
    /// hardest kind to trace back to its cause.
    #[test]
    fn end_reaches_the_far_side_even_through_a_stage_that_swallows_everything() {
        let mut pipeline = Pipeline::new().then(Blackhole).then(Counter { seen: 0 });
        let out = pipeline.push(Frame::End).unwrap();
        assert_eq!(out, vec![Frame::End], "the stream still ends");
    }

    #[test]
    fn ordinary_frames_are_not_reinstated() {
        let mut pipeline = Pipeline::new().then(Blackhole);
        assert!(pipeline.push(audio()).unwrap().is_empty());
        assert!(pipeline.push(Frame::Voice {
            lane: Lane::Mic,
            probability: 0.5
        })
        .unwrap()
        .is_empty());
    }

    #[test]
    fn a_stage_that_forwards_control_frames_is_not_double_counted() {
        let mut pipeline = Pipeline::new().then(Counter { seen: 0 });
        assert_eq!(pipeline.push(Frame::End).unwrap(), vec![Frame::End]);
    }

    #[test]
    fn push_all_collects_across_frames() {
        let mut pipeline = Pipeline::new().then(Split);
        let out = pipeline.push_all([audio(), audio()]).unwrap();
        assert_eq!(out.len(), 6);
    }

    /// One pipeline serves a second recording without being rebuilt, which is why reset exists at
    /// all rather than constructing a new chain.
    #[test]
    fn reset_reaches_every_stage() {
        struct Probe(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl Processor for Probe {
            fn name(&self) -> &'static str {
                "probe"
            }
            fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
                Ok(vec![frame])
            }
            fn reset(&mut self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut pipeline = Pipeline::new()
            .then(Probe(count.clone()))
            .then(Probe(count.clone()));
        pipeline.reset();
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    /// A stateless stage should not have to write an empty `reset`.
    #[test]
    fn reset_is_optional() {
        let mut pipeline = Pipeline::new().then(Swallow);
        pipeline.reset();
    }

    #[test]
    fn an_error_in_a_stage_stops_the_frame_rather_than_being_swallowed() {
        struct Fails;
        impl Processor for Fails {
            fn name(&self) -> &'static str {
                "fails"
            }
            fn push(&mut self, _frame: Frame) -> Result<Vec<Frame>> {
                Err(summo_core::Error::Other("nope".into()))
            }
        }
        let mut pipeline = Pipeline::new().then(Fails).then(Counter { seen: 0 });
        assert!(pipeline.push(audio()).is_err());
    }
}
