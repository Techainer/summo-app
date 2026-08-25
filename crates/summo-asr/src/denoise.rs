//! Taking the room off the recording before the decoder hears it.
//!
//! `Task::Denoise` has been in the manifest enum since the enum was written, and until now it named
//! nothing: no model in the registry, no runtime, no stage in the pipeline. The only code that ever
//! matched on it turned it into the words "noise suppression" for a label nobody could reach. This
//! is the runtime it was named for.
//!
//! ## Where it runs, and why not earlier
//!
//! On a **closed utterance**, immediately before the final decode — [`crate::PseudoSession::finalize`]
//! — and nowhere else.
//!
//! sherpa-onnx exposes GTCRN through an *offline* entry point: hand it a whole array, get a whole
//! array back. GTCRN is causal underneath and the wrapper carries state across the frames of one
//! call, so a call per utterance is the shape the API was built for. A call per 30 ms audio frame
//! would restart that state sixty times a second, and the model would spend every call in its own
//! warm-up — the artefacts that produces are worse than the noise it removes.
//!
//! That rules out the placement somebody would reach for first, ahead of the voice detector. It
//! also costs less than it sounds: the detector's job is deciding *whether* somebody is speaking,
//! which Silero does well enough in a noisy room, and the decoder's job is deciding *what they
//! said*, which is where noise actually costs words.
//!
//! Partials are left alone. A partial re-decodes a growing window every few hundred milliseconds,
//! so denoising there would re-clean the same audio a dozen times for text that is about to be
//! replaced by the final anyway.
//!
//! ## What it costs
//!
//! Measured on the published export, one thread: real-time factor **0.102**, so a three second
//! utterance takes about three hundred milliseconds. That runs where the final decode already runs
//! — the frame loop — and roughly triples how long finalising a sentence takes beside a model like
//! Gipformer at 0.06. It is a real cost and it is the reason the number is in the manifest rather
//! than in a comment: the recommendation engine treats an unmeasured model as slow, and this one
//! deserves to be judged on the figure somebody took.
//!
//! More threads make it worse — 0.115 at two, 0.114 at four, 0.120 at eight. The model is small and
//! causal; there is little to divide and the synchronisation costs more than it saves. The loader
//! asks for one and ignores the machine's recommendation, which is the opposite of what every other
//! runtime here wants.
//!
//! ## Why it is off unless asked for
//!
//! A speech enhancer is not free accuracy. On clean speech — one person, a good microphone, a quiet
//! room — GTCRN removes things that were not noise and the word error rate goes *up*. Turning this
//! on for everybody by default would improve the meetings held next to an air conditioner and
//! quietly damage the ones held in an office, and the people whose transcripts got worse would have
//! no way to know why. So it is a model you install and choose, like every other model here.

use summo_core::Result;

/// Something that takes noise out of one utterance's audio.
///
/// One method, because there is one question. Nothing here streams: see the module note on why the
/// unit of work is an utterance rather than a frame.
pub trait Denoiser: Send {
    /// Clean one utterance.
    ///
    /// The result is the same length and sample rate as the input, so a segment's `t0` and `t1`
    /// still describe the audio the decoder is about to see. Implementations whose runtime works in
    /// frames are responsible for that; see [`Gtcrn::denoise`], which is short by a frame and says
    /// so.
    ///
    /// # Errors
    ///
    /// When the underlying runtime fails. A caller should keep the original audio and carry on —
    /// noise is a worse transcript, and a failed clean-up is not a reason to lose the utterance.
    fn denoise(&mut self, pcm: &[f32]) -> Result<Vec<f32>>;

    /// The registry id, for logs and `/status`.
    fn name(&self) -> &str;
}

#[cfg(feature = "sherpa")]
pub use gtcrn::Gtcrn;

#[cfg(feature = "sherpa")]
mod gtcrn {
    use std::ffi::{CString, c_int};

    use sherpa_rs::sherpa_rs_sys as sys;
    use summo_core::{Error, Result, audio::SAMPLE_RATE};

    use super::Denoiser;

    /// GTCRN, through sherpa-onnx's C API.
    ///
    /// Raw FFI rather than a `sherpa-rs` wrapper because there is no wrapper: `sherpa-rs` 0.6 binds
    /// the recognisers, the taggers and the TTS voices, and stops short of the speech enhancer. The
    /// C entry points are in the generated `sherpa-rs-sys` bindings all the same, so the choice is
    /// between these forty lines and no noise suppression.
    pub struct Gtcrn {
        handle: *const sys::SherpaOnnxOfflineSpeechDenoiser,
        /// Kept so the model's own rate can be checked once at load rather than per utterance.
        rate: u32,
        name: String,
    }

    // The handle is used behind `&mut self` from one thread at a time — the pipeline owns it, the
    // same as a decoder. sherpa's denoiser holds inference state, so this is `Send` and not `Sync`.
    unsafe impl Send for Gtcrn {}

    impl Gtcrn {
        /// Load the model at `path`.
        ///
        /// # Errors
        ///
        /// When the file is missing or is not a GTCRN export, or when the model works at a sample
        /// rate this pipeline does not produce.
        pub fn load(path: &str, threads: u32, name: &str) -> Result<Self> {
            let model = CString::new(path).map_err(|_| {
                Error::Config(format!("model path is not usable as a C string: {path}"))
            })?;
            let provider = CString::new("cpu").expect("a literal with no interior nul");

            let config = sys::SherpaOnnxOfflineSpeechDenoiserConfig {
                model: sys::SherpaOnnxOfflineSpeechDenoiserModelConfig {
                    gtcrn: sys::SherpaOnnxOfflineSpeechDenoiserGtcrnModelConfig {
                        model: model.as_ptr(),
                    },
                    num_threads: threads.clamp(1, 16) as c_int,
                    debug: 0,
                    provider: provider.as_ptr(),
                },
            };

            // SAFETY: `config` borrows two `CString`s that outlive this call, and sherpa copies
            // every field it keeps. A null return is the documented failure, not undefined
            // behaviour.
            let handle = unsafe { sys::SherpaOnnxCreateOfflineSpeechDenoiser(&config) };
            if handle.is_null() {
                return Err(Error::Config(format!(
                    "`{name}` could not be loaded as a speech enhancer from {path}"
                )));
            }

            // SAFETY: `handle` is non-null and was just returned by the constructor.
            let rate = unsafe { sys::SherpaOnnxOfflineSpeechDenoiserGetSampleRate(handle) };
            let rate = u32::try_from(rate).unwrap_or(0);
            if rate != SAMPLE_RATE {
                // SAFETY: `handle` is non-null and owned here; nothing else has seen it.
                unsafe { sys::SherpaOnnxDestroyOfflineSpeechDenoiser(handle) };
                return Err(Error::Config(format!(
                    "`{name}` works at {rate} Hz and this pipeline records at {SAMPLE_RATE} Hz"
                )));
            }

            Ok(Self {
                handle,
                rate,
                name: name.to_string(),
            })
        }

        /// The rate the model was built for. Equal to [`SAMPLE_RATE`] or the load would have failed.
        #[must_use]
        pub fn sample_rate(&self) -> u32 {
            self.rate
        }
    }

    impl Denoiser for Gtcrn {
        fn denoise(&mut self, pcm: &[f32]) -> Result<Vec<f32>> {
            // An empty utterance cannot reach here through the gate, but a caller is entitled to
            // ask, and sherpa is entitled to dislike a null pointer with a length of zero.
            if pcm.is_empty() {
                return Ok(Vec::new());
            }
            let n = c_int::try_from(pcm.len()).map_err(|_| {
                Error::Other(format!(
                    "{} samples is more than one call can carry",
                    pcm.len()
                ))
            })?;

            // SAFETY: `handle` is non-null for the life of `self`, and `pcm` is a live slice of `n`
            // floats for the duration of the call.
            let out = unsafe {
                sys::SherpaOnnxOfflineSpeechDenoiserRun(
                    self.handle,
                    pcm.as_ptr(),
                    n,
                    self.rate as c_int,
                )
            };
            if out.is_null() {
                return Err(Error::Other(format!("`{}` returned no audio", self.name)));
            }

            // SAFETY: non-null, and sherpa guarantees `samples` points at `n` floats until the
            // matching destroy call below.
            let mut cleaned = unsafe {
                let view = &*out;
                let len = usize::try_from(view.n).unwrap_or(0);
                let samples = if len == 0 || view.samples.is_null() {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(view.samples, len).to_vec()
                };
                sys::SherpaOnnxDestroyDenoisedAudio(out);
                samples
            };

            // GTCRN works in frames and returns whole ones, so the answer is short by up to a frame
            // — a second of input measured 23 808 samples back rather than 24 000. Twelve
            // milliseconds off the end of an utterance is inaudible and is nearly always the gate's
            // trailing hangover rather than a word, but "nearly always" is not a contract, and the
            // segment's `t1` was computed from the original length. So the tail is restored from the
            // audio it was taken from: un-denoised for those few milliseconds, and exactly as long
            // as it says it is.
            //
            // Bounded, because reconciling *any* difference would hide the failure this guards
            // against. A frame is 16 ms; a model handing back an array a different length than that
            // is not doing framing, it is the wrong model or a resampler, and every timestamp
            // downstream would be wrong in a way no assertion later could attribute.
            const SLACK: usize = SAMPLE_RATE as usize / 32; // 32 ms, two frames and a margin.
            if cleaned.len().abs_diff(pcm.len()) > SLACK {
                return Err(Error::Other(format!(
                    "`{}` returned {} samples for {} — not a framing remainder",
                    self.name,
                    cleaned.len(),
                    pcm.len()
                )));
            }
            if cleaned.len() < pcm.len() {
                cleaned.extend_from_slice(&pcm[cleaned.len()..]);
            } else {
                cleaned.truncate(pcm.len());
            }

            Ok(cleaned)
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    impl Drop for Gtcrn {
        fn drop(&mut self) {
            // SAFETY: non-null for the life of `self`, and this runs once.
            unsafe { sys::SherpaOnnxDestroyOfflineSpeechDenoiser(self.handle) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The real GTCRN export. Every claim this file makes is about what the model does, so
        /// there is nothing here worth asserting against a stub.
        fn model() -> Option<String> {
            std::env::var("SUMMO_TEST_DENOISE_MODEL").ok()
        }

        fn energy(pcm: &[f32]) -> f32 {
            if pcm.is_empty() {
                return 0.0;
            }
            pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32
        }

        /// Deterministic noise. `Math.random`'s Rust equivalent would make a flaky assertion out of
        /// a decidable one, and a linear congruential generator is enough to sound like hiss.
        fn hiss(n: usize) -> Vec<f32> {
            let mut state: u32 = 0x2545_f491;
            (0..n)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 8) as f32 / f32::from(u16::MAX) / 128.0 - 0.1
                })
                .collect()
        }

        #[test]
        fn a_missing_model_is_reported_rather_than_crashed_into() {
            let Some(_) = model() else {
                eprintln!("skipping: set SUMMO_TEST_DENOISE_MODEL");
                return;
            };
            assert!(Gtcrn::load("/nonexistent/gtcrn.onnx", 1, "gtcrn").is_err());
        }

        /// The reason the feature exists. A second of hiss with no speech in it should come back
        /// quieter than it went in — that is the whole claim, and it is the one a fake would let
        /// through untested.
        #[test]
        fn a_room_with_nobody_in_it_comes_back_quieter() {
            let Some(path) = model() else {
                eprintln!("skipping: set SUMMO_TEST_DENOISE_MODEL");
                return;
            };
            let mut gtcrn = Gtcrn::load(&path, 2, "gtcrn").unwrap();

            let noisy = hiss(SAMPLE_RATE as usize);
            let clean = gtcrn.denoise(&noisy).unwrap();

            let (before, after) = (energy(&noisy), energy(&clean));
            assert!(
                after < before * 0.5,
                "noise energy {before} -> {after}, which is not suppression"
            );
        }

        /// The pipeline puts the result back where the original was: same rate, same length, so the
        /// segment's `t0`/`t1` still describe the audio the decoder sees. A model that returned a
        /// resampled or trimmed array would move every timestamp in the meeting.
        #[test]
        fn the_utterance_keeps_its_length() {
            let Some(path) = model() else {
                eprintln!("skipping: set SUMMO_TEST_DENOISE_MODEL");
                return;
            };
            let mut gtcrn = Gtcrn::load(&path, 2, "gtcrn").unwrap();
            assert_eq!(gtcrn.sample_rate(), SAMPLE_RATE);

            let noisy = hiss(SAMPLE_RATE as usize * 3 / 2);
            let clean = gtcrn.denoise(&noisy).unwrap();
            assert_eq!(clean.len(), noisy.len());
        }

        /// Nothing in is nothing out, without reaching the C API at all. The gate cannot produce an
        /// empty utterance, but `Denoiser` is a public trait and a null pointer with a length of
        /// zero is not a thing to find out about in production.
        /// Not an assertion — a measurement, printed so the registry manifest can quote a number
        /// somebody took rather than a number somebody hoped for.
        #[test]
        #[ignore = "measurement, not a check"]
        fn measure_real_time_factor() {
            let Some(path) = model() else { return };
            for threads in [1u32, 2, 4, 8] {
                let mut gtcrn = Gtcrn::load(&path, threads, "gtcrn").unwrap();
                let audio = hiss(SAMPLE_RATE as usize * 3);
                // One warm call; the first touches lazily-allocated arenas.
                let _ = gtcrn.denoise(&audio).unwrap();
                let started = std::time::Instant::now();
                for _ in 0..10 {
                    let _ = gtcrn.denoise(&audio).unwrap();
                }
                let per = started.elapsed().as_secs_f64() / 10.0;
                eprintln!(
                    "{threads}t: 3 s utterance in {per:.3} s -> rtf {:.4}",
                    per / 3.0
                );
            }
        }

        #[test]
        fn nothing_in_is_nothing_out() {
            let Some(path) = model() else {
                eprintln!("skipping: set SUMMO_TEST_DENOISE_MODEL");
                return;
            };
            let mut gtcrn = Gtcrn::load(&path, 1, "gtcrn").unwrap();
            assert!(gtcrn.denoise(&[]).unwrap().is_empty());
        }
    }
}
