//! Silero VAD backend (MIT).
//!
//! Silero is the default because it is genuinely permissively licensed, runs anywhere ONNX Runtime
//! runs, and is accurate enough that segmentation quality is dominated by [`crate::gate`] tuning
//! rather than by the detector. Its weakness is release latency — it holds "speech" for a moment
//! after speech actually stops — which the gate compensates for and which
//! [`docs/adr/0001-vad-backend-licensing.md`] discusses.
//!
//! Two checkpoint generations are in circulation and both are supported, detected from the graph's
//! input names rather than from a version the caller has to supply:
//!
//! * v4: `x`, `h`, `c` → `prob`, `new_h`, `new_c` (two LSTM states of shape `[2, 1, 64]`)
//! * v5: `input`, `state`, `sr` → `output`, `stateN` (one packed state of shape `[2, 1, 128]`)
//!
//! The v5 graph additionally expects the caller to prepend [`CONTEXT_LEN`] samples of the previous
//! frame, so its real input is 576 wide even though the hop is still 512. Feeding it a bare 512
//! samples does not error — it silently returns ≈0.001 for every frame, i.e. "no speech, ever".
//! That failure mode is invisible without a labelled benchmark, which is how it was caught here.

use std::path::Path;

use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use summo_core::{Error, Result, audio::SAMPLE_RATE};

use crate::Vad;

/// Silero consumes exactly 512 samples (32 ms at 16 kHz) per call.
pub const FRAME_LEN: usize = 512;

/// Samples of the previous frame that v5 graphs expect to be prepended to the current one.
pub const CONTEXT_LEN: usize = 64;

/// Which checkpoint generation the loaded graph is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    /// Separate `h` and `c` LSTM states.
    V4,
    /// Packed `state` plus a sample-rate input.
    V5,
}

pub struct SileroVad {
    session: Session,
    flavor: Flavor,
    /// v4: `h`. v5: the packed `state`.
    state_a: Vec<f32>,
    /// v4: `c`. Unused on v5.
    state_b: Vec<f32>,
    /// v5 only: the tail of the previous frame, prepended to the next one.
    context: Vec<f32>,
    /// v5 only: reusable `[context | frame]` buffer, so the hot path does not allocate.
    input_buf: Vec<f32>,
    input_name: String,
    output_name: String,
}

impl SileroVad {
    /// Load a Silero checkpoint.
    ///
    /// `threads` is clamped to 1 in practice — the model is tiny and extra threads cost more in
    /// synchronisation than they recover, while stealing cores from the ASR decoder that needs them.
    pub fn load(model: impl AsRef<Path>, threads: usize) -> Result<Self> {
        let path = model.as_ref();
        let session = build_session(path, threads)
            .map_err(|e| Error::Vad(format!("cannot load {}: {e}", path.display())))?;

        let inputs: Vec<String> = session
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        let outputs: Vec<String> = session
            .outputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();

        let flavor = if inputs.iter().any(|n| n == "h") && inputs.iter().any(|n| n == "c") {
            Flavor::V4
        } else if inputs.iter().any(|n| n == "state") {
            Flavor::V5
        } else {
            return Err(Error::Vad(format!(
                "unrecognised Silero graph: inputs {inputs:?}, outputs {outputs:?}"
            )));
        };

        let input_name = inputs
            .iter()
            .find(|n| *n == "x" || *n == "input")
            .cloned()
            .ok_or_else(|| Error::Vad(format!("no audio input in {inputs:?}")))?;
        let output_name = outputs
            .iter()
            .find(|n| *n == "prob" || *n == "output")
            .cloned()
            .ok_or_else(|| Error::Vad(format!("no probability output in {outputs:?}")))?;

        let (a, b) = match flavor {
            Flavor::V4 => (vec![0.0; 2 * 64], vec![0.0; 2 * 64]),
            Flavor::V5 => (vec![0.0; 2 * 128], Vec::new()),
        };

        Ok(Self {
            session,
            flavor,
            state_a: a,
            state_b: b,
            context: vec![0.0; CONTEXT_LEN],
            input_buf: Vec::with_capacity(CONTEXT_LEN + FRAME_LEN),
            input_name,
            output_name,
        })
    }
}

/// Session construction, split out because each `ort` builder step returns a distinct error type
/// that only unifies through a boxed trait object.
fn build_session(
    path: &Path,
    threads: usize,
) -> std::result::Result<Session, Box<dyn std::error::Error>> {
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        // The model is tiny; extra threads cost more in synchronisation than they recover, and
        // steal cores from the ASR decoder that actually needs them.
        .with_intra_threads(threads.clamp(1, 2))?;
    Ok(builder.commit_from_file(path)?)
}

impl Vad for SileroVad {
    fn frame_len(&self) -> usize {
        FRAME_LEN
    }

    fn feed_frame(&mut self, frame: &[f32]) -> Result<f32> {
        if frame.len() != FRAME_LEN {
            return Err(Error::Vad(format!(
                "Silero needs exactly {FRAME_LEN} samples, got {}",
                frame.len()
            )));
        }

        let outputs = match self.flavor {
            Flavor::V4 => {
                let x = Tensor::from_array(([1_usize, FRAME_LEN], frame.to_vec()))
                    .map_err(|e| Error::Vad(e.to_string()))?;
                let h = Tensor::from_array(([2_usize, 1, 64], self.state_a.clone()))
                    .map_err(|e| Error::Vad(e.to_string()))?;
                let c = Tensor::from_array(([2_usize, 1, 64], self.state_b.clone()))
                    .map_err(|e| Error::Vad(e.to_string()))?;
                self.session
                    .run(ort::inputs! {
                        self.input_name.clone() => x,
                        "h" => h,
                        "c" => c,
                    })
                    .map_err(|e| Error::Vad(e.to_string()))?
            }
            Flavor::V5 => {
                self.input_buf.clear();
                self.input_buf.extend_from_slice(&self.context);
                self.input_buf.extend_from_slice(frame);
                let x = Tensor::from_array((
                    [1_usize, CONTEXT_LEN + FRAME_LEN],
                    self.input_buf.clone(),
                ))
                .map_err(|e| Error::Vad(e.to_string()))?;
                let state = Tensor::from_array(([2_usize, 1, 128], self.state_a.clone()))
                    .map_err(|e| Error::Vad(e.to_string()))?;
                let sr = Tensor::from_array(((), vec![i64::from(SAMPLE_RATE)]))
                    .map_err(|e| Error::Vad(e.to_string()))?;
                self.session
                    .run(ort::inputs! {
                        self.input_name.clone() => x,
                        "state" => state,
                        "sr" => sr,
                    })
                    .map_err(|e| Error::Vad(e.to_string()))?
            }
        };

        let prob = outputs[self.output_name.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Vad(e.to_string()))?
            .1
            .first()
            .copied()
            .ok_or_else(|| Error::Vad("empty probability output".into()))?;

        // Carry recurrent state forward; without this every frame is judged in isolation and the
        // detector flickers.
        match self.flavor {
            Flavor::V4 => {
                let h = outputs["new_h"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::Vad(e.to_string()))?;
                let c = outputs["new_c"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::Vad(e.to_string()))?;
                self.state_a.copy_from_slice(h.1);
                self.state_b.copy_from_slice(c.1);
            }
            Flavor::V5 => {
                let s = outputs["stateN"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::Vad(e.to_string()))?;
                self.state_a.copy_from_slice(s.1);
                self.context
                    .copy_from_slice(&frame[FRAME_LEN - CONTEXT_LEN..]);
            }
        }

        Ok(prob.clamp(0.0, 1.0))
    }

    fn reset(&mut self) {
        self.state_a.fill(0.0);
        self.state_b.fill(0.0);
        self.context.fill(0.0);
    }

    fn name(&self) -> &'static str {
        "silero"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to a checkpoint for tests. Set `SUMMO_TEST_SILERO` to run these against a real model;
    /// they are skipped otherwise so the suite stays runnable without downloads.
    fn model_path() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_SILERO").map(std::path::PathBuf::from)
    }

    #[test]
    fn wrong_frame_length_is_rejected() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_SILERO to a silero .onnx");
            return;
        };
        let mut vad = SileroVad::load(path, 1).unwrap();
        assert!(vad.feed_frame(&[0.0; 256]).is_err());
    }

    #[test]
    fn silence_scores_below_speech() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_SILERO to a silero .onnx");
            return;
        };
        let mut vad = SileroVad::load(path, 1).unwrap();

        let silence = [0.0_f32; FRAME_LEN];
        let mut silence_prob = 0.0;
        for _ in 0..10 {
            silence_prob = vad.feed_frame(&silence).unwrap();
        }

        // A 200 Hz sawtooth is not speech, but it is voiced-ish energy: the point of this check is
        // only that the model responds to signal at all rather than returning a constant.
        vad.reset();
        let tone: Vec<f32> = (0..FRAME_LEN)
            .map(|i| ((i % 80) as f32 / 40.0) - 1.0)
            .collect();
        let mut tone_prob = 0.0;
        for _ in 0..10 {
            tone_prob = vad.feed_frame(&tone).unwrap();
        }

        assert!(silence_prob < 0.2, "silence scored {silence_prob}");
        assert!(
            tone_prob > silence_prob,
            "tone {tone_prob} vs silence {silence_prob}"
        );
    }

    #[test]
    fn reset_restores_initial_behaviour() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_SILERO to a silero .onnx");
            return;
        };
        let mut vad = SileroVad::load(path, 1).unwrap();
        let silence = [0.0_f32; FRAME_LEN];

        let first = vad.feed_frame(&silence).unwrap();
        for _ in 0..20 {
            vad.feed_frame(&silence).unwrap();
        }
        vad.reset();
        let after_reset = vad.feed_frame(&silence).unwrap();

        assert!(
            (first - after_reset).abs() < 1e-6,
            "reset must clear recurrent state: {first} vs {after_reset}"
        );
    }
}
