//! `summo dub` — a meeting, spoken in another language, over its own recording.
//!
//! Every piece of this existed and nothing joined them: translations on disk, a fitting plan, a
//! synthesiser, a mixer. This is the command that runs them in the one order that works.
//!
//! ## Two passes, and why
//!
//! Fitting a line into its slot needs to know how long the line takes to say, and the only way to
//! know that is to say it. So: synthesise everything at natural speed, plan against the real
//! durations, then synthesise again at the speed the plan chose.
//!
//! The second pass is not waste. Asking the model for a shorter line produces speech *at* that
//! length rather than speech that has been sped up — measured: resampling a 1.3× line raises its
//! zero-crossing rate about 9%, regenerating it leaves the rate alone. One extra pass costs
//! seconds; the alternative costs the pitch of every line in the meeting.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use summo_core::paths::Paths;
use summo_tts::{
    Synthesizer,
    dub::{Mix, Take},
    plan::{Fit, Line},
};

pub struct Options {
    pub meeting: String,
    pub lang: String,
    /// A registry id, or a directory. See [`resolve_voice`].
    pub voice: String,
    pub out: PathBuf,
    /// Gain for the original recording under the dub. 0.0 removes it.
    pub under: f32,
    pub threads: usize,
}

/// Where the voice's files are: an installed registry model, or a directory somebody unpacked.
///
/// The id first, and a path only when the id is not one. A registry id cannot contain a path
/// separator, so the two cannot be confused — and trying the store first means `--voice
/// vits-vi-vais1000` works on a machine where a directory of that name happens to exist in the
/// working directory, which is the reading somebody typing an id intends.
fn resolve_voice(paths: &Paths, wanted: &str) -> Result<PathBuf> {
    if let Ok(id) = summo_core::ModelId::parse(wanted) {
        let store = summo_models::ModelStore::new(paths.clone());
        if let Ok(manifest) = store.installed(&id) {
            if manifest.task != summo_models::Task::Tts {
                bail!(
                    "`{wanted}` is a {} model, not a voice",
                    summo_models::page::task_name(manifest.task)
                );
            }
            let installed = store.resolve(&manifest)?;
            // `dir` points inside the archive the voice ships as: a piper voice is an `.onnx` plus
            // several hundred phoneme tables, and `Vits::load` is given the folder holding them.
            return installed
                .param_dir("dir")
                .or_else(|| installed.files.values().next().cloned())
                .with_context(|| format!("`{wanted}` is installed but has no voice directory"));
        }
    }

    let path = PathBuf::from(wanted);
    if path.is_dir() {
        return Ok(path);
    }
    bail!(
        "no voice `{wanted}`: not an installed model, and not a directory.          Install one with `summo pull vits-vi-vais1000`."
    )
}

pub fn run(paths: &Paths, opts: &Options) -> Result<()> {
    // The voice first, before the meeting and the translation are read. A mistyped `--voice` used
    // to be reported after all of that, which on a long meeting is a wait for an answer that was
    // available immediately.
    let voice = resolve_voice(paths, &opts.voice)?;

    let id = summo_core::MeetingId::from(opts.meeting.clone());

    let path = summo_engine::summarize::find_meeting_file(&paths.vault(), &id)
        .with_context(|| format!("no meeting {}", opts.meeting))?;
    let doc = summo_vault::open(&paths.vault(), &path)?;

    let translation =
        summo_vault::translation::load(paths, &id, &opts.lang)?.with_context(|| {
            format!(
                "meeting {} has no {} translation — run `summo translate` first",
                opts.meeting, opts.lang
            )
        })?;

    // Only lines that were translated. An untranslated one keeps its original audio underneath,
    // which is a better answer than speaking the source language in the target voice.
    let lines: Vec<(u64, f64, f64, String)> = doc
        .transcript
        .iter()
        .filter_map(|s| {
            translation
                .get(s.seq)
                .map(|text| (s.seq, s.t0, s.t1, text.to_string()))
        })
        .collect();

    if lines.is_empty() {
        bail!("nothing translated to dub");
    }

    let mut tts = summo_tts::vits::Vits::load(&voice, opts.threads)?;
    println!("voice  {}", voice.display());
    println!(
        "lines  {} of {} translated",
        lines.len(),
        doc.transcript.len()
    );

    // Pass one: how long does each line take at natural speed?
    let mut measured = Vec::with_capacity(lines.len());
    for (seq, t0, t1, text) in &lines {
        let speech = tts.say_at(text, 1.0)?;
        measured.push(Line {
            seq: *seq,
            text: text.clone(),
            t0: *t0,
            t1: *t1,
            spoken_s: speech.duration_s(),
        });
    }

    let total_s = doc.frontmatter.duration as f64;
    let plan = summo_tts::plan(&measured, total_s);

    // Pass two: say each line at the speed its slot needs.
    let mut takes = Vec::with_capacity(plan.slots.len());
    let mut rate = tts.rate();
    for (slot, line) in plan.slots.iter().zip(&measured) {
        let speech = tts.say_at(&line.text, slot.speed as f32)?;
        rate = speech.rate;
        takes.push(Take {
            seq: slot.seq,
            samples: speech.samples,
        });
    }

    // The plan's speeds were applied by the model, so nothing here should stretch again.
    let flat = summo_tts::plan::Plan {
        slots: plan
            .slots
            .iter()
            .map(|s| summo_tts::plan::Slot {
                speed: 1.0,
                ..s.clone()
            })
            .collect(),
        ..plan.clone()
    };

    let under = load_under(paths, &id, rate);
    let track = summo_tts::dub::assemble(
        &flat,
        &takes,
        &under,
        rate,
        Mix {
            under_gain: opts.under,
            voice_gain: 1.0,
        },
    );

    summo_tts::dub::write_wav(&opts.out, &track, rate)?;

    let overflows = plan.slots.iter().filter(|s| s.fit == Fit::Overflow).count();
    println!(
        "wrote  {} ({:.1}s at {rate} Hz)\nfit    {} natural, {} adjusted, {} overflowing{}",
        opts.out.display(),
        track.len() as f64 / f64::from(rate.max(1)),
        plan.slots.iter().filter(|s| s.fit == Fit::Natural).count(),
        plan.slots.iter().filter(|s| s.fit == Fit::Adjusted).count(),
        overflows,
        if overflows > 0 {
            format!(" — worst runs {:.1}s long", plan.worst_over_s)
        } else {
            String::new()
        }
    );
    Ok(())
}

/// The original recording, to sit under the dub. Absent is fine — the dub stands alone.
fn load_under(paths: &Paths, id: &summo_core::MeetingId, rate: u32) -> Vec<f32> {
    let dir = paths.audio_for(id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let Some(wav) = entries
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("wav"))
    else {
        return Vec::new();
    };

    match read_wav_at(&wav, rate) {
        Ok(samples) => samples,
        // A bed that will not load costs the bed, not the dub.
        Err(e) => {
            tracing::warn!(file = %wav.display(), error = %e, "no original under the dub");
            Vec::new()
        }
    }
}

/// Read a WAV, nearest-neighbour resampled to `rate`.
///
/// Nearest-neighbour is enough here and only here: this is a bed mixed at 18% gain under speech,
/// where resampling artefacts are inaudible. Anything the user listens to directly goes through
/// ffmpeg.
fn read_wav_at(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .filter_map(Result::ok)
            .map(|v| f32::from(v) / 32_768.0)
            .collect(),
    };

    // Downmix first: a stereo original played as mono at the wrong stride is a chipmunk bed.
    let mono: Vec<f32> = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    } else {
        samples
    };

    if spec.sample_rate == rate || mono.is_empty() {
        return Ok(mono);
    }
    let ratio = f64::from(spec.sample_rate) / f64::from(rate.max(1));
    let out_len = (mono.len() as f64 / ratio) as usize;
    Ok((0..out_len)
        .map(|i| mono[((i as f64 * ratio) as usize).min(mono.len() - 1)])
        .collect())
}
