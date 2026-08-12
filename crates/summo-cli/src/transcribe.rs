//! `summo transcribe` — run the real pipeline over a file.
//!
//! This is the end-to-end check that the pieces fit: WAV in, VAD segmentation, pseudo-streaming
//! decode, hallucination filtering, transcript out. It is also where the real-time factor gets
//! measured against a whole recording rather than a synthetic clip, which is the number that
//! decides whether a model is usable live on a given machine.

use std::{path::Path, time::Instant};

use anyhow::{Context, Result, bail};
use summo_asr::{
    Decoder, PseudoSession, SessionConfig,
    sherpa::{SenseVoiceDecoder, WhisperDecoder, ZipformerDecoder},
};
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

/// Which runtime loads the model directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Engine {
    /// Zipformer RNN-T. Fast, single-language, does not invent text.
    Transducer,
    /// Whisper. 99 languages and code-switching, at the cost of hallucinating over silence.
    Whisper,
    /// SenseVoice. Chinese, Japanese, Korean, Cantonese and English, non-autoregressive — so it is
    /// fast and, unlike Whisper, does not invent an ending for a half-spoken sentence.
    SenseVoice,
}

pub struct Options {
    pub audio: std::path::PathBuf,
    pub model_dir: std::path::PathBuf,
    pub vad_model: std::path::PathBuf,
    pub engine: Engine,
    /// ISO language code for Whisper; `None` asks it to detect.
    pub language: Option<String>,
    pub threads: usize,
    pub partial_step_ms: u32,
    /// Print partials as they are produced, to see the live behaviour rather than just the result.
    pub show_partials: bool,
    /// Run the same models through a `summo_pipeline` chain instead of the hand-written loop.
    ///
    /// Here so the two can be compared on identical audio. Swapping the transcription path on an
    /// argument rather than on a measurement is how a working pipeline stops working quietly.
    pub pipeline: bool,
}

pub fn run(opts: &Options) -> Result<()> {
    let pcm = read_wav(&opts.audio)?;
    let audio_secs = samples_to_secs(pcm.len());

    let load_start = Instant::now();
    // Boxed once: both paths take the trait object, so the comparison runs the same detector.
    let mut vad: Box<dyn Vad> = Box::new(SileroVad::load(&opts.vad_model, 1)?);
    let decoder: Box<dyn Decoder> = match opts.engine {
        Engine::Transducer => Box::new(ZipformerDecoder::from_dir(&opts.model_dir, opts.threads)?),
        Engine::Whisper => Box::new(WhisperDecoder::from_dir(
            &opts.model_dir,
            opts.language.as_deref(),
            opts.threads,
        )?),
        Engine::SenseVoice => Box::new(SenseVoiceDecoder::from_dir(
            &opts.model_dir,
            opts.language.as_deref(),
            opts.threads,
        )?),
    };
    let model_name = decoder.name().to_string();
    let partials_supported = decoder.supports_partials();
    let load_ms = load_start.elapsed().as_millis();

    let cfg = SessionConfig {
        partial_step_ms: opts.partial_step_ms,
        lane: Lane::Mic,
        ..SessionConfig::default()
    };
    let mut session = PseudoSession::new(decoder, cfg);

    println!(
        "audio  {} ({audio_secs:.1}s)\nmodel  {model_name} ({} threads, loaded in {load_ms} ms){}\nvad    {}\n",
        opts.audio.display(),
        opts.threads,
        if partials_supported {
            ""
        } else {
            " — final-only, this model does not do partials"
        },
        vad.name()
    );

    let frame_len = vad.frame_len();
    let start = Instant::now();
    let mut finals = 0;

    // Counted before the session is handed off: the pipeline path takes ownership of it, and both
    // paths should report the same numbers from the same place.
    let (events, decodes, suppressed) = if opts.pipeline {
        // The same models, assembled as a chain rather than a hand-written loop. Here so the two
        // can be compared on identical audio: a transcription path swapped on an argument rather
        // than on a measurement is how a working pipeline stops working quietly.
        pipeline_events(vad, session, &pcm, opts)?
    } else {
        let events = hand_written_events(&mut vad, &mut session, &pcm, frame_len, opts)?;
        let counts = (session.decode_count(), session.suppressed_count());
        (events, counts.0, counts.1)
    };

    for event in events {
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

    let wall = start.elapsed();
    let rtf = wall.as_secs_f64() / audio_secs.max(f64::EPSILON);

    println!(
        "\n{finals} segment(s) · {} decode(s) · {} suppressed\n\
         wall {:.2}s for {audio_secs:.1}s of audio → RTF {rtf:.4} ({:.0}× real time)",
        decodes,
        suppressed,
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

/// The original loop: one frame at a time, straight into the session.
fn hand_written_events(
    vad: &mut Box<dyn Vad>,
    session: &mut PseudoSession<Box<dyn Decoder>>,
    pcm: &[f32],
    frame_len: usize,
    _opts: &Options,
) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    // Fed exactly as live audio arrives: one frame at a time, in order.
    for frame in pcm.chunks_exact(frame_len) {
        let prob = vad.feed_frame(frame)?;
        out.extend(session.accept(frame, prob)?);
    }
    out.extend(session.flush()?);
    Ok(out)
}

/// The same work as a `summo_pipeline` chain.
///
/// Audio goes in as one block rather than pre-cut frames — the reframer is a stage now, which is
/// the point: the caller no longer has to know what width the detector wants.
fn pipeline_events(
    vad: Box<dyn Vad>,
    session: PseudoSession<Box<dyn Decoder>>,
    pcm: &[f32],
    _opts: &Options,
) -> Result<(Vec<Event>, u64, u64)> {
    use summo_engine::stages::{Detect, Recognise};
    use summo_pipeline::{Frame, Pipeline, processors::Reframe};

    let width = vad.frame_len();
    let mut pipeline = Pipeline::new()
        .then(Reframe::new(width, 16_000))
        .then(Detect::new(Lane::Mic, vad))
        .then(Recognise::from_session(Lane::Mic, session));

    let mut frames = pipeline.push(Frame::audio(Lane::Mic, pcm.to_vec(), 16_000))?;
    frames.extend(pipeline.push(Frame::End)?);

    let events: Vec<Event> = frames
        .into_iter()
        .filter_map(|f| match f {
            Frame::Event(event) => Some(event),
            _ => None,
        })
        .collect();

    // The counters live on the stage now, which is where the session went.
    let counts = pipeline
        .stage::<Recognise>()
        .map_or((0, 0), |r| (r.decode_count(), r.suppressed_count()));
    Ok((events, counts.0, counts.1))
}
