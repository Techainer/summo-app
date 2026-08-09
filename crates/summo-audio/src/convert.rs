//! Getting device audio into the one format everything downstream expects.
//!
//! Devices deliver whatever they like: 44.1 or 48 kHz, one to eight channels, interleaved. The VAD
//! and every model want 16 kHz mono `f32`. Converting once, here, is what lets the rest of the
//! codebase ignore the question entirely.
//!
//! The resampler is fed from a real-time audio callback, so it must not allocate per call and must
//! tolerate arbitrary input chunk sizes — a device may hand over 480 samples or 4096, and it may
//! change mid-stream.

use rubato::{FftFixedIn, Resampler};
use summo_core::{Error, Result, audio::SAMPLE_RATE};

/// Downmix interleaved frames to mono by averaging channels.
///
/// Averaging rather than taking channel 0: a stereo USB microphone often has the useful signal on
/// one side only, and dropping the other half loses 3 dB for no reason.
pub fn to_mono(interleaved: &[f32], channels: u16, out: &mut Vec<f32>) {
    let channels = channels.max(1) as usize;
    out.clear();
    if channels == 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    out.reserve(interleaved.len() / channels);
    for frame in interleaved.chunks_exact(channels) {
        out.push(frame.iter().sum::<f32>() / channels as f32);
    }
}

/// Resamples an arbitrary input rate to [`SAMPLE_RATE`], accepting variable chunk sizes.
pub struct Resampling {
    inner: Option<FftFixedIn<f32>>,
    /// Samples the resampler consumes per call.
    chunk: usize,
    /// Input not yet forming a whole chunk.
    pending: Vec<f32>,
    /// Scratch buffers, kept to avoid allocating in the audio callback.
    in_buf: Vec<Vec<f32>>,
    out_buf: Vec<Vec<f32>>,
    source_rate: u32,
}

impl Resampling {
    /// Build a converter from `source_rate`. A source already at [`SAMPLE_RATE`] passes through.
    pub fn new(source_rate: u32) -> Result<Self> {
        if source_rate == 0 {
            return Err(Error::Audio("device reported a sample rate of 0".into()));
        }
        if source_rate == SAMPLE_RATE {
            return Ok(Self {
                inner: None,
                chunk: 0,
                pending: Vec::new(),
                in_buf: Vec::new(),
                out_buf: Vec::new(),
                source_rate,
            });
        }

        let chunk = aligned_chunk(source_rate);
        let inner = FftFixedIn::<f32>::new(source_rate as usize, SAMPLE_RATE as usize, chunk, 1, 1)
            .map_err(|e| Error::Audio(format!("cannot resample {source_rate} Hz: {e}")))?;

        let out_capacity = inner.output_frames_max();
        Ok(Self {
            inner: Some(inner),
            chunk,
            pending: Vec::with_capacity(chunk * 2),
            in_buf: vec![vec![0.0; chunk]],
            out_buf: vec![vec![0.0; out_capacity]],
            source_rate,
        })
    }

    #[must_use]
    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    #[must_use]
    pub fn is_passthrough(&self) -> bool {
        self.inner.is_none()
    }

    /// Convert mono input, appending 16 kHz output to `out`.
    ///
    /// Input that does not fill a whole chunk is buffered until it does, so output arrives slightly
    /// behind input — bounded by one chunk, about 21 ms at 48 kHz.
    pub fn process(&mut self, mono: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let Some(resampler) = self.inner.as_mut() else {
            out.extend_from_slice(mono);
            return Ok(());
        };

        self.pending.extend_from_slice(mono);
        while self.pending.len() >= self.chunk {
            self.in_buf[0].copy_from_slice(&self.pending[..self.chunk]);
            self.pending.drain(..self.chunk);

            let (_, written) = resampler
                .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                .map_err(|e| Error::Audio(format!("resample failed: {e}")))?;
            out.extend_from_slice(&self.out_buf[0][..written]);
        }
        Ok(())
    }

    /// Samples held back waiting for a full chunk.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// Choose an input chunk size the FFT resampler can convert without leaving a remainder.
///
/// The transform maps a whole number of input samples to a whole number of output samples, and that
/// ratio is `source / gcd(source, 16000)` in to `16000 / gcd` out. A chunk that is not a multiple of
/// the input step leaves a fractional remainder at every chunk boundary, and the discontinuities
/// that produces are audible as buzz — a 100 Hz tone came back with 152 zero crossings per second
/// instead of 100 before this was aligned.
///
/// Targets roughly 1024 samples (about 21 ms at 48 kHz): short enough not to add noticeable latency,
/// long enough for the FFT to be efficient.
fn aligned_chunk(source_rate: u32) -> usize {
    const TARGET: usize = 1024;
    let step = (source_rate / gcd(source_rate, SAMPLE_RATE)).max(1) as usize;
    let multiples = (TARGET / step).max(1);
    step * multiples
}

const fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Splits a continuous sample stream into fixed-size frames.
///
/// Every consumer downstream — VAD backends especially — requires an exact frame length, while
/// audio arrives in whatever sizes the device chooses.
pub struct Framer {
    frame_len: usize,
    buf: Vec<f32>,
}

impl Framer {
    #[must_use]
    pub fn new(frame_len: usize) -> Self {
        Self {
            frame_len: frame_len.max(1),
            buf: Vec::with_capacity(frame_len * 4),
        }
    }

    /// Append samples and invoke `on_frame` for each complete frame produced.
    pub fn push<F: FnMut(&[f32])>(&mut self, samples: &[f32], mut on_frame: F) {
        self.buf.extend_from_slice(samples);
        let mut consumed = 0;
        while self.buf.len() - consumed >= self.frame_len {
            on_frame(&self.buf[consumed..consumed + self.frame_len]);
            consumed += self.frame_len;
        }
        if consumed > 0 {
            self.buf.drain(..consumed);
        }
    }

    /// Emit whatever is left, zero-padded to a whole frame. Used at end of stream only.
    pub fn flush<F: FnMut(&[f32])>(&mut self, mut on_frame: F) {
        if self.buf.is_empty() {
            return;
        }
        self.buf.resize(self.frame_len, 0.0);
        on_frame(&self.buf);
        self.buf.clear();
    }

    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_input_passes_through_unchanged() {
        let mut out = Vec::new();
        to_mono(&[0.1, 0.2, 0.3], 1, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn stereo_is_averaged_not_truncated() {
        // A microphone with signal on one channel only must not lose half its level.
        let mut out = Vec::new();
        to_mono(&[1.0, 0.0, 0.5, 0.5], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn a_ragged_tail_is_dropped_rather_than_misaligned() {
        let mut out = Vec::new();
        to_mono(&[1.0, 1.0, 1.0], 2, &mut out);
        assert_eq!(out.len(), 1, "an incomplete frame cannot be downmixed");
    }

    #[test]
    fn matching_rate_is_a_passthrough() {
        let mut r = Resampling::new(SAMPLE_RATE).unwrap();
        assert!(r.is_passthrough());

        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3], &mut out).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn a_zero_sample_rate_is_rejected() {
        assert!(Resampling::new(0).is_err());
    }

    #[test]
    fn resampling_48k_yields_about_a_third_as_many_samples() {
        let mut r = Resampling::new(48_000).unwrap();
        assert!(!r.is_passthrough());

        // One second of 48 kHz audio.
        let input = vec![0.0_f32; 48_000];
        let mut out = Vec::new();
        r.process(&input, &mut out).unwrap();

        // Allow for the chunk still buffered at the end.
        let expected = SAMPLE_RATE as usize;
        assert!(
            out.len() as i64 > expected as i64 - 2_000 && out.len() <= expected,
            "expected about {expected} samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resampling_survives_irregular_chunk_sizes() {
        // Devices do not promise a stable buffer size, and some change it mid-stream.
        let mut r = Resampling::new(44_100).unwrap();
        let mut out = Vec::new();
        for size in [64_usize, 480, 1024, 37, 4096, 128] {
            r.process(&vec![0.0; size], &mut out).unwrap();
        }
        assert!(!out.is_empty());
        assert!(r.pending() < 1024, "backlog must stay bounded by one chunk");
    }

    #[test]
    fn chunk_sizes_divide_evenly_into_the_conversion_ratio() {
        for rate in [
            8_000_u32, 22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000,
        ] {
            let chunk = aligned_chunk(rate);
            let step = (rate / gcd(rate, SAMPLE_RATE)) as usize;
            assert_eq!(
                chunk % step,
                0,
                "chunk {chunk} for {rate} Hz leaves a remainder against step {step}"
            );
            assert!(chunk > 0);
        }
    }

    #[test]
    fn a_sine_keeps_its_frequency_through_resampling() {
        // Guards against an off-by-one in chunking that would change pitch: count zero crossings of
        // a 100 Hz tone before and after.
        let src = 48_000_f32;
        let input: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 / src * 100.0 * std::f32::consts::TAU).sin())
            .collect();

        let mut r = Resampling::new(48_000).unwrap();
        let mut out = Vec::new();
        r.process(&input, &mut out).unwrap();

        // Count rising zero crossings with a small guard band, so numerical noise around zero is
        // not mistaken for a cycle.
        let crossings = out
            .windows(2)
            .filter(|w| w[0] <= -0.01 && w[1] > 0.01)
            .count();
        assert!(
            (93..=103).contains(&crossings),
            "expected ~100 cycles after resampling, counted {crossings}"
        );
    }

    #[test]
    fn framer_emits_only_complete_frames() {
        let mut framer = Framer::new(100);
        let mut lengths = Vec::new();

        framer.push(&vec![0.0; 250], |f| lengths.push(f.len()));
        assert_eq!(lengths, vec![100, 100]);
        assert_eq!(framer.buffered(), 50);

        framer.push(&[0.0; 50], |f| lengths.push(f.len()));
        assert_eq!(lengths, vec![100, 100, 100]);
        assert_eq!(framer.buffered(), 0);
    }

    #[test]
    fn framer_preserves_sample_order_across_pushes() {
        let mut framer = Framer::new(4);
        let mut frames: Vec<Vec<f32>> = Vec::new();

        framer.push(&[1.0, 2.0, 3.0], |f| frames.push(f.to_vec()));
        framer.push(&[4.0, 5.0, 6.0, 7.0, 8.0], |f| frames.push(f.to_vec()));

        assert_eq!(
            frames,
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]]
        );
    }

    #[test]
    fn flush_pads_the_final_partial_frame() {
        let mut framer = Framer::new(4);
        let mut frames: Vec<Vec<f32>> = Vec::new();

        framer.push(&[1.0, 2.0], |f| frames.push(f.to_vec()));
        assert!(frames.is_empty());

        framer.flush(|f| frames.push(f.to_vec()));
        assert_eq!(frames, vec![vec![1.0, 2.0, 0.0, 0.0]]);
    }

    #[test]
    fn flushing_an_empty_framer_emits_nothing() {
        let mut framer = Framer::new(4);
        let mut calls = 0;
        framer.flush(|_| calls += 1);
        assert_eq!(calls, 0);
    }
}
