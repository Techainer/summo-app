//! Audio constants shared by capture, VAD and ASR.
//!
//! Everything downstream of capture speaks one format: 16 kHz, mono, `f32` in `[-1.0, 1.0]`.
//! Resampling happens once, at the capture boundary, so no other crate has to care about the
//! device's native rate.

/// The single sample rate used across the whole pipeline.
pub const SAMPLE_RATE: u32 = 16_000;

/// Wire frame duration. Matches the 100 ms cadence proven out in the Python prototype: small
/// enough that partial latency is dominated by decode, large enough that IPC overhead is noise.
pub const FRAME_MS: u32 = 100;

/// Samples per wire frame (1600 at 16 kHz / 100 ms).
pub const FRAME_LEN: usize = (SAMPLE_RATE as usize * FRAME_MS as usize) / 1000;

/// Convert a sample count at [`SAMPLE_RATE`] to seconds.
#[inline]
#[must_use]
pub fn samples_to_secs(samples: usize) -> f64 {
    samples as f64 / f64::from(SAMPLE_RATE)
}

/// Convert seconds to a sample count at [`SAMPLE_RATE`], rounding to nearest.
#[inline]
#[must_use]
pub fn secs_to_samples(secs: f64) -> usize {
    (secs * f64::from(SAMPLE_RATE)).round().max(0.0) as usize
}

/// Convert milliseconds to a sample count at [`SAMPLE_RATE`].
#[inline]
#[must_use]
pub fn ms_to_samples(ms: u32) -> usize {
    (SAMPLE_RATE as usize * ms as usize) / 1000
}

/// Root-mean-square level of a PCM slice, used for the waveform HUD and mic-level checks.
#[must_use]
pub fn rms(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f32 = pcm.iter().map(|s| s * s).sum();
    (sum / pcm.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_len_is_100ms() {
        assert_eq!(FRAME_LEN, 1600);
        assert!((samples_to_secs(FRAME_LEN) - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn sample_conversions_round_trip() {
        for ms in [10_u32, 16, 100, 400, 500, 700] {
            let samples = ms_to_samples(ms);
            assert_eq!(samples, secs_to_samples(f64::from(ms) / 1000.0));
        }
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0; 32]), 0.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_full_scale_square_is_one() {
        let pcm: Vec<f32> = (0..64)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert!((rms(&pcm) - 1.0).abs() < 1e-6);
    }
}
