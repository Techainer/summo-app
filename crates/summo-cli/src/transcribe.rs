//! `summo transcribe` — run the real pipeline over a file.
//!
//! This is the end-to-end check that the pieces fit: WAV in, VAD segmentation, pseudo-streaming
//! decode, hallucination filtering, transcript out. It is also where the real-time factor gets
//! measured against a whole recording rather than a synthetic clip, which is the number that
//! decides whether a model is usable live on a given machine.

use std::{path::Path, time::Instant};

use anyhow::{Context, Result, bail};
use summo_asr::{Decoder, PseudoSession, SessionConfig, sherpa::ZipformerDecoder};
use summo_core::{
    Event,
    audio::{SAMPLE_RATE, samples_to_secs},
    segment::Lane,
};
use summo_vad::{Vad, silero::SileroVad};

/// Read a mono WAV at the pipeline's sample rate.
fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        bail!(
            "{}: expected mono, got {} channels",
            path.display(),
            spec.channels
        );
    }
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "{}: expected {SAMPLE_RATE} Hz, got {}. Resample it first — transcribing at the wrong \
             rate produces confident nonsense rather than an error.",
            path.display(),
            spec.sample_rate
        );
    }

    Ok(match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / f32::from(i16::MAX);
            reader
                .samples::<i16>()
                .map(|s| s.map(|v| f32::from(v) * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
    })
}

pub struct Options {
    pub audio: std::path::PathBuf,
    pub model_dir: std::path::PathBuf,
    pub vad_model: std::path::PathBuf,
    pub threads: usize,
    pub partial_step_ms: u32,
    /// Print partials as they are produced, to see the live behaviour rather than just the result.
    pub show_partials: bool,
}

pub fn run(opts: &Options) -> Result<()> {
    let pcm = read_wav(&opts.audio)?;
    let audio_secs = samples_to_secs(pcm.len());

    let load_start = Instant::now();
    let mut vad = SileroVad::load(&opts.vad_model, 1)?;
    let decoder = ZipformerDecoder::from_dir(&opts.model_dir, opts.threads)?;
    let model_name = decoder.name().to_string();
    let load_ms = load_start.elapsed().as_millis();

    let cfg = SessionConfig {
        partial_step_ms: opts.partial_step_ms,
        lane: Lane::Mic,
        ..SessionConfig::default()
    };
    let mut session = PseudoSession::new(decoder, cfg);

    println!(
        "audio  {} ({audio_secs:.1}s)\nmodel  {model_name} ({} threads, loaded in {load_ms} ms)\nvad    {}\n",
        opts.audio.display(),
        opts.threads,
        vad.name()
    );

    let frame_len = vad.frame_len();
    let start = Instant::now();
    let mut finals = 0;

    // Feed the file exactly as live audio would arrive: one frame at a time, in order.
    for frame in pcm.chunks_exact(frame_len) {
        let prob = vad.feed_frame(frame)?;
        for event in session.accept(frame, prob)? {
            match event {
                Event::Final(seg) => {
                    finals += 1;
                    println!("[{:>7.2}s → {:>7.2}s] {}", seg.t0, seg.t1, seg.text);
                }
                Event::Partial(seg) if opts.show_partials => {
                    println!("  … {:.2}s  {}", seg.t1, seg.text);
                }
                _ => {}
            }
        }
    }
    for event in session.flush()? {
        if let Event::Final(seg) = event {
            finals += 1;
            println!("[{:>7.2}s → {:>7.2}s] {}", seg.t0, seg.t1, seg.text);
        }
    }

    let wall = start.elapsed();
    let rtf = wall.as_secs_f64() / audio_secs.max(f64::EPSILON);

    println!(
        "\n{finals} segment(s) · {} decode(s) · {} suppressed\n\
         wall {:.2}s for {audio_secs:.1}s of audio → RTF {rtf:.4} ({:.0}× real time)",
        session.decode_count(),
        session.suppressed_count(),
        wall.as_secs_f64(),
        1.0 / rtf.max(f64::EPSILON),
    );

    // The pseudo-streaming loop only works while there is headroom; past 1.0 the app would fall
    // behind live audio and the backlog would grow without bound.
    if rtf >= 1.0 {
        println!(
            "\nWARNING: RTF {rtf:.2} is at or above real time. This model cannot keep up live on \
             this machine — choose a smaller one or raise --partial-step-ms."
        );
    }
    Ok(())
}
