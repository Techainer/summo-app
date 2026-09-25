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
    /// A registry id, or a directory. `None` takes the chosen voice, then the only installed one.
    /// See [`resolve_voice`].
    pub voice: Option<String>,
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
///
/// `lang` is the language the dub is in, and it is checked against the voice. A VITS voice does not
/// fail on a language it was not trained for: it runs the text through the phoneme table it has and
/// says whatever comes out. A Vietnamese voice handed an English line produces confident nonsense,
/// and a lexicon-based Chinese one logs `OOV` per word and emits a tenth of a second — both of which
/// this wrote over the recording without a word until the comparison below existed.
fn resolve_voice(paths: &Paths, wanted: Option<&str>, lang: &str) -> Result<PathBuf> {
    let store = summo_models::ModelStore::new(paths.clone());
    let wanted = match wanted.map(str::trim).filter(|w| !w.is_empty()) {
        Some(wanted) => wanted.to_string(),
        None => chosen_voice(paths, &store, lang)?,
    };
    let wanted = wanted.as_str();

    if let Ok(id) = summo_core::ModelId::parse(wanted)
        && let Ok(manifest) = store.installed(&id)
    {
        if manifest.task != summo_models::Task::Tts {
            bail!(
                "`{wanted}` is a {} model, not a voice",
                summo_models::page::task_name(manifest.task)
            );
        }
        if !summo_models::langs_cover(&manifest.langs, lang) {
            bail!(
                "`{wanted}` speaks {}, not {lang}.{}",
                spoken(&manifest.langs),
                alternatives(&installed_voices(&store), lang)
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

    // A directory carries no manifest, so there is nothing to compare `lang` against. Left
    // unchecked rather than guessed at from the folder's name: `--voice /path` is the escape hatch
    // for somebody running a voice they trained, and they know what it speaks.
    let path = PathBuf::from(wanted);
    if path.is_dir() {
        return Ok(path);
    }
    // One space after the full stop. The literal was wrapped by hand and the indentation went into
    // the string, so every user who mistyped `--voice` got ten spaces in the middle of the sentence.
    bail!(
        "no voice `{wanted}`: not an installed model, and not a directory. `summo registry ls` \
         lists the voices there are to pull."
    )
}

/// Every installed voice, manifest and all.
fn installed_voices(store: &summo_models::ModelStore) -> Vec<summo_models::Manifest> {
    store
        .list()
        .into_iter()
        .filter(|m| m.task == summo_models::Task::Tts)
        .collect()
}

/// The voice to use when none was named on the command line.
///
/// Only voices that speak `lang` are candidates. A voice that cannot say the line is not a choice
/// between, it is a wrong answer, so the screen's preference is honoured **among those** rather
/// than over them: `models.tts` first when it covers the language, then the only remaining voice.
/// Several is still a question rather than a silent pick — but publishing an English and a Chinese
/// voice beside the Vietnamese one made "several are installed" the normal state, and filtering by
/// language is what keeps that from turning every dub into a prompt.
fn chosen_voice(paths: &Paths, store: &summo_models::ModelStore, lang: &str) -> Result<String> {
    let settings = summo_core::Settings::load(&paths.settings()).unwrap_or_default();
    pick_voice(
        &installed_voices(store),
        settings.models.tts.as_deref(),
        lang,
    )
}

/// The decision inside [`chosen_voice`], with the disk taken out of it.
///
/// Separated so it can be tested. Choosing a voice is three rules interacting — a preference, a
/// language, and how many are installed — and the version of this that read `Paths` could only be
/// exercised by installing a voice, which is why it shipped with none of the cases checked.
fn pick_voice(
    installed: &[summo_models::Manifest],
    preferred: Option<&str>,
    lang: &str,
) -> Result<String> {
    let preferred = preferred.map(str::trim).filter(|id| !id.is_empty());

    let speaks: Vec<&summo_models::Manifest> = installed
        .iter()
        .filter(|m| summo_models::langs_cover(&m.langs, lang))
        .collect();

    if let Some(id) = preferred {
        // Not installed, or installed and unreadable: say nothing and let `resolve_voice` report
        // it. A stale id in settings is its problem to name, not this function's.
        let known = installed.iter().any(|m| m.id.as_str() == id);
        if !known || speaks.iter().any(|m| m.id.as_str() == id) {
            return Ok(id.to_string());
        }
    }

    match speaks.as_slice() {
        [] if installed.is_empty() => bail!(
            "no voice installed. `summo registry ls` lists them; pull one that speaks {lang}, or \
             pass --voice with a directory."
        ),
        [] => bail!("no installed voice speaks {lang}.{}", spoken_by(installed)),
        [only] => Ok(only.id.to_string()),
        several => bail!(
            "several installed voices speak {lang} ({}). Choose one on the models screen, or pass \
             --voice.",
            several
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The languages a manifest claims, as a reader would say them.
fn spoken(langs: &[String]) -> String {
    if langs.iter().any(|l| l == "*") {
        return "every language".into();
    }
    if langs.is_empty() {
        return "no declared language".into();
    }
    langs.join(", ")
}

/// What is installed and what each one speaks, for an error that has to name the way out.
fn spoken_by(installed: &[summo_models::Manifest]) -> String {
    if installed.is_empty() {
        return String::new();
    }
    format!(
        " Installed: {}. `summo registry ls` lists the voices there are to pull.",
        installed
            .iter()
            .map(|m| format!("{} ({})", m.id, spoken(&m.langs)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The installed voices that *would* have worked, appended to a refusal.
fn alternatives(installed: &[summo_models::Manifest], lang: &str) -> String {
    let speaks: Vec<String> = installed
        .iter()
        .filter(|m| summo_models::langs_cover(&m.langs, lang))
        .map(|m| m.id.to_string())
        .collect();
    if speaks.is_empty() {
        return spoken_by(installed);
    }
    format!(" Installed and speaking {lang}: {}.", speaks.join(", "))
}

pub fn run(paths: &Paths, opts: &Options) -> Result<()> {
    // The voice first, before the meeting and the translation are read. A mistyped `--voice` used
    // to be reported after all of that, which on a long meeting is a wait for an answer that was
    // available immediately.
    let voice = resolve_voice(paths, opts.voice.as_deref(), &opts.lang)?;

    let id = summo_core::MeetingId::from(opts.meeting.clone());

    let path = summo_engine::summarize::find_meeting_file(&paths.vault(), &id)
        .with_context(|| format!("no meeting {}", opts.meeting))?;
    let doc = summo_vault::open(&paths.vault(), &path)?;

    // The way out has to be one that exists. This told the reader to run a translate subcommand,
    // and there has never been one under any feature — translating a finished meeting is a daemon
    // route, reached from the meeting's export panel. So the prerequisite for the one command that
    // needs it named a command nobody could run. See `summo-core`'s errors_name_real_commands.
    let translation =
        summo_vault::translation::load(paths, &id, &opts.lang)?.with_context(|| {
            format!(
                "meeting {} has no {} translation yet. Open it in Summo and translate it into \
                 {} first — the language list is in the meeting's export panel.",
                opts.meeting, opts.lang, opts.lang
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

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(id: &str, langs: &[&str]) -> summo_models::Manifest {
        let json = serde_json::json!({
            "schema": 1,
            "id": id,
            "name": id,
            "task": "tts",
            "mode": "batch",
            "runtime": "sherpa-onnx/vits",
            "langs": langs,
            "license": "MIT",
            "attribution": "nobody",
            "files": [{
                "name": "voice.tar.bz2",
                "sha256": "a".repeat(64),
                "size": 1,
                "url": "https://example.invalid/voice.tar.bz2",
                "archive": "tar-bz2"
            }],
            "params": {"dir": "voice.tar.bz2/voice"}
        });
        summo_models::Manifest::parse(&json.to_string()).unwrap()
    }

    /// The bug this whole comparison exists for.
    ///
    /// One voice installed and it speaks Vietnamese; the dub is English. Before, "the only installed
    /// voice" was the answer and a Vietnamese phoneme table read the English aloud.
    #[test]
    fn the_only_voice_is_not_the_answer_when_it_speaks_another_language() {
        let installed = [voice("vits-vi-vais1000", &["vi"])];
        let err = pick_voice(&installed, None, "en").unwrap_err().to_string();
        assert!(err.contains("no installed voice speaks en"), "{err}");
        // An error about a missing voice has to name what is there, or the reader's next move is a
        // guess.
        assert!(err.contains("vits-vi-vais1000 (vi)"), "{err}");
    }

    #[test]
    fn the_only_voice_that_speaks_the_language_needs_no_asking() {
        let installed = [
            voice("vits-vi-vais1000", &["vi"]),
            voice("vits-en-ljspeech", &["en"]),
        ];
        assert_eq!(
            pick_voice(&installed, None, "en").unwrap(),
            "vits-en-ljspeech"
        );
        assert_eq!(
            pick_voice(&installed, None, "vi").unwrap(),
            "vits-vi-vais1000"
        );
    }

    /// The screen's choice is honoured among the voices that can do the job, not over them.
    ///
    /// A preference is a preference between candidates. Reading an English line in a Vietnamese
    /// voice because a screen once said Vietnamese is not honouring a choice, it is producing
    /// nonsense on the strength of one.
    #[test]
    fn a_preference_for_a_voice_that_cannot_say_the_line_does_not_win() {
        let installed = [
            voice("vits-vi-vais1000", &["vi"]),
            voice("vits-en-ljspeech", &["en"]),
        ];
        let chosen = pick_voice(&installed, Some("vits-vi-vais1000"), "en").unwrap();
        assert_eq!(chosen, "vits-en-ljspeech");
        // And it does win where it applies.
        let chosen = pick_voice(&installed, Some("vits-vi-vais1000"), "vi").unwrap();
        assert_eq!(chosen, "vits-vi-vais1000");
    }

    /// A stale id in settings is reported by the resolver, which knows it is not installed. Swallowing
    /// it here would turn "you chose a voice that is gone" into a different voice speaking.
    #[test]
    fn a_preference_naming_nothing_installed_is_passed_through_to_be_reported() {
        let installed = [voice("vits-en-ljspeech", &["en"])];
        let chosen = pick_voice(&installed, Some("vits-gone"), "en").unwrap();
        assert_eq!(chosen, "vits-gone");
    }

    #[test]
    fn several_voices_for_one_language_is_a_question() {
        let installed = [
            voice("vits-en-ljspeech", &["en"]),
            voice("vits-en-other", &["en"]),
        ];
        let err = pick_voice(&installed, None, "en").unwrap_err().to_string();
        assert!(err.contains("vits-en-ljspeech, vits-en-other"), "{err}");
    }

    /// `langs: ["*"]` is a claim to every language, and `langs_cover` honours it. A multilingual
    /// voice must not be filtered out of its own job.
    #[test]
    fn a_voice_claiming_every_language_covers_the_one_asked_for() {
        let installed = [voice("vits-multi", &["*"])];
        assert_eq!(pick_voice(&installed, None, "ja").unwrap(), "vits-multi");
    }

    /// `en-US` asks for `en`.
    #[test]
    fn a_regional_code_matches_the_language_it_belongs_to() {
        let installed = [voice("vits-en-ljspeech", &["en"])];
        assert_eq!(
            pick_voice(&installed, None, "en-US").unwrap(),
            "vits-en-ljspeech"
        );
    }

    #[test]
    fn nothing_installed_says_so_rather_than_listing_an_empty_set() {
        let err = pick_voice(&[], None, "en").unwrap_err().to_string();
        assert!(err.contains("no voice installed"), "{err}");
        assert!(err.contains("speaks en"), "{err}");
    }
}
