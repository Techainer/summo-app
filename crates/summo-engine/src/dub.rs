//! A meeting, spoken in another language, over its own recording.
//!
//! Every piece of this existed and nothing joined them: translations on disk, a fitting plan, a
//! synthesiser, a mixer. This is the code that runs them in the one order that works.
//!
//! ## Why it lives in the daemon
//!
//! It was a subcommand, and only a subcommand. `summo dub` worked, had tests, and two voices were
//! published for it — and nothing in the app could reach it: no route, no screen, not one string in
//! the catalogue. A feature the product page lists under "what works today" and that a user cannot
//! find is, from where they are standing, a feature that does not exist.
//!
//! So the pipeline moved here and the command became a caller. One implementation, two front doors.
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
//!
//! ## It is a job, not a request
//!
//! Minutes of synthesis for a long meeting, exactly like an import — so the caller gets an id and
//! asks how it is going, and a dub started from the command line is visible to a screen that
//! connects later. See [`Dubs`], which is [`crate::imports::Imports`] with a different payload.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use summo_core::{MeetingId, paths::Paths};
use summo_tts::{
    Synthesizer,
    dub::{Mix, Take},
    plan::{Fit, Line},
};

/// The file name a finished dub is stored under, inside the meeting's audio directory.
///
/// `dub-vi.wav`, beside `mic.opus` and `import.wav`. One function rather than a format string in
/// three files, because [`crate::audio_stream::locate`] has to recognise exactly what this writes —
/// and the last time a track was written under a name the player did not know about, every imported
/// meeting drew a player whose only lane answered `no such lane`.
#[must_use]
pub fn lane_name(lang: &str) -> String {
    format!("dub-{}", normalise_lang(lang))
}

/// Where a dub for `lang` goes.
#[must_use]
pub fn default_out(paths: &Paths, meeting: &MeetingId, lang: &str) -> PathBuf {
    paths
        .audio_for(meeting)
        .join(format!("{}.wav", lane_name(lang)))
}

/// A language tag reduced to something that is safe in a file name and stable across callers.
///
/// Lowercased, and anything that is not a letter, a digit or a hyphen dropped. This is the only
/// place a caller-supplied string gets near a path in this module, and `..` and `/` do not survive
/// it. [`crate::audio_stream`] applies the same rule from the other side, so a lane the player asks
/// for and a file this writes cannot disagree.
#[must_use]
pub fn normalise_lang(lang: &str) -> String {
    lang.trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect()
}

/// The languages this meeting already has a dub in, newest naming aside, sorted.
#[must_use]
pub fn languages(paths: &Paths, meeting: &MeetingId) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(paths.audio_for(meeting))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".wav")
                .and_then(|stem| stem.strip_prefix("dub-"))
                .map(str::to_string)
        })
        .filter(|lang| !lang.is_empty())
        .collect();
    found.sort();
    found.dedup();
    found
}

pub struct Options {
    pub meeting: String,
    pub lang: String,
    /// A registry id, or a directory. `None` takes the chosen voice, then the only one that speaks
    /// the language. See [`resolve_voice`].
    pub voice: Option<String>,
    /// Where to write. `None` puts it beside the recording, as the lane the player can switch to —
    /// which is what the app wants, and what the command line wants only when it says so.
    pub out: Option<PathBuf>,
    /// Gain for the original recording under the dub. 0.0 removes it.
    pub under: f32,
    pub threads: usize,
}

/// What a finished dub turned out to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The voice directory that spoke it, for a line of output that says which.
    pub voice: String,
    /// Lines dubbed, and lines in the meeting. They differ when a translation is partial, which is
    /// normal and worth seeing.
    pub lines: usize,
    pub of: usize,
    pub out: String,
    pub duration_s: f64,
    pub rate: u32,
    pub natural: usize,
    pub adjusted: usize,
    pub overflowing: usize,
    pub worst_over_s: f64,
}

/// Where a dub job is.
///
/// The two synthesis passes are reported separately rather than averaged into one bar. They are the
/// same work twice and a bar that fills, resets and fills again looks broken; a bar that says which
/// pass it is on does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum JobState {
    /// Accepted, not yet started.
    Queued,
    /// Reading the meeting and loading the voice. A voice is several hundred files and takes a
    /// visible moment; a bar stuck at zero during it reads as a hang.
    Loading,
    /// Saying the lines. `pass` is 1 or 2 — see the module docs for why there are two.
    ///
    /// `spoken` rather than `done`, because `done` is also the name of the state this ends in and a
    /// client reading `job.done` would have two meanings to tell apart at the one moment it matters.
    Speaking {
        pass: u8,
        spoken: usize,
        total: usize,
    },
    /// Laying the takes over the recording and writing the file.
    Mixing,
    Done {
        #[serde(flatten)]
        report: Report,
    },
    Failed {
        error: String,
    },
}

impl JobState {
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self, JobState::Done { .. } | JobState::Failed { .. })
    }

    /// Fraction complete, for a progress bar. `None` while there is nothing honest to show.
    ///
    /// Each pass is half the bar, so it fills once across work that happens twice.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        match self {
            JobState::Speaking {
                pass,
                spoken,
                total,
            } if *total > 0 => {
                let within = *spoken as f64 / *total as f64;
                let offset = if *pass >= 2 { 0.5 } else { 0.0 };
                Some((offset + within / 2.0).clamp(0.0, 1.0))
            }
            JobState::Mixing => Some(0.98),
            JobState::Done { .. } => Some(1.0),
            _ => None,
        }
    }
}

/// One dub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// The meeting being dubbed, so a screen showing one meeting can filter to its own jobs.
    pub meeting: String,
    /// What the user will recognise it by — the meeting's title.
    pub title: String,
    pub lang: String,
    #[serde(flatten)]
    pub state: JobState,
}

/// Every dub this daemon has run since it started.
///
/// Not persisted, for the same reason [`crate::imports::Imports`] is not: a job interrupted by a
/// restart is gone, and recording its final state would promise a resume that does not exist. The
/// *file* a finished job produced is on disk, and that is what the player looks for.
#[derive(Clone, Default)]
pub struct Dubs {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    order: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Debug for Dubs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dubs")
            .field("jobs", &self.order.lock().len())
            .finish()
    }
}

impl Dubs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a job and return its id.
    pub fn add(&self, meeting: &str, title: impl Into<String>, lang: &str) -> String {
        let id = MeetingId::new().to_string();
        let job = Job {
            id: id.clone(),
            meeting: meeting.to_string(),
            title: title.into(),
            lang: lang.to_string(),
            state: JobState::Queued,
        };
        self.jobs.lock().insert(id.clone(), job);
        self.order.lock().push(id.clone());
        id
    }

    pub fn set(&self, id: &str, state: JobState) {
        if let Some(job) = self.jobs.lock().get_mut(id) {
            job.state = state;
        }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().get(id).cloned()
    }

    /// Every job, newest first.
    #[must_use]
    pub fn list(&self) -> Vec<Job> {
        let jobs = self.jobs.lock();
        self.order
            .lock()
            .iter()
            .rev()
            .filter_map(|id| jobs.get(id).cloned())
            .collect()
    }

    /// Whether anything is still working, so a screen polls only while it needs to.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.jobs.lock().values().any(|j| !j.state.is_finished())
    }

    /// Drop finished jobs, returning how many went.
    pub fn clear_finished(&self) -> usize {
        let mut jobs = self.jobs.lock();
        let mut order = self.order.lock();
        let before = jobs.len();
        jobs.retain(|_, j| !j.state.is_finished());
        order.retain(|id| jobs.contains_key(id));
        before - jobs.len()
    }
}

/// Run a dub as a registered job, recording where it got to and why it stopped.
///
/// **Failure is recorded, not swallowed.** A job that dies leaves a [`JobState::Failed`] with the
/// message, because a job that quietly disappears is indistinguishable from one still working.
pub fn run_job(dubs: &Dubs, id: &str, paths: &Paths, opts: &Options) {
    let progress = |state: JobState| dubs.set(id, state);
    match run(paths, opts, &progress) {
        Ok(report) => dubs.set(id, JobState::Done { report }),
        Err(error) => dubs.set(
            id,
            JobState::Failed {
                // `{error:#}` rather than `{error}`: anyhow prints only the outermost context by
                // default, and the outermost context here is usually the least specific thing that
                // went wrong.
                error: format!("{error:#}"),
            },
        ),
    }
}

/// Synthesise a meeting's translation over its own recording.
///
/// `progress` is called as the passes advance. It is a plain closure rather than a channel so the
/// command line can print to a terminal and the daemon can write into a job registry without this
/// function knowing which.
///
/// # Errors
///
/// When the voice cannot be resolved, the meeting is not found, there is no translation in `lang`,
/// or nothing in the meeting was translated.
pub fn run(paths: &Paths, opts: &Options, progress: &dyn Fn(JobState)) -> Result<Report> {
    // The voice first, before the meeting and the translation are read. A mistyped `--voice` used
    // to be reported after all of that, which on a long meeting is a wait for an answer that was
    // available immediately.
    let voice = resolve_voice(paths, opts.voice.as_deref(), &opts.lang)?;

    progress(JobState::Loading);

    let id = MeetingId::from(opts.meeting.clone());

    let path = crate::summarize::find_meeting_file(&paths.vault(), &id)
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
    let total = lines.len();

    // Pass one: how long does each line take at natural speed?
    let mut measured = Vec::with_capacity(total);
    for (spoken, (seq, t0, t1, text)) in lines.iter().enumerate() {
        progress(JobState::Speaking {
            pass: 1,
            spoken,
            total,
        });
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
    for (spoken, (slot, line)) in plan.slots.iter().zip(&measured).enumerate() {
        progress(JobState::Speaking {
            pass: 2,
            spoken,
            total,
        });
        let speech = tts.say_at(&line.text, slot.speed as f32)?;
        rate = speech.rate;
        takes.push(Take {
            seq: slot.seq,
            samples: speech.samples,
        });
    }

    progress(JobState::Mixing);

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

    let out = match &opts.out {
        Some(explicit) => explicit.clone(),
        None => default_out(paths, &id, &opts.lang),
    };
    // The audio directory exists for a recorded meeting and may not for one whose audio was pruned.
    // Creating it is the difference between a dub and a refusal a user cannot act on.
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    summo_tts::dub::write_wav(&out, &track, rate)?;

    Ok(Report {
        voice: voice.display().to_string(),
        lines: total,
        of: doc.transcript.len(),
        out: out.display().to_string(),
        duration_s: track.len() as f64 / f64::from(rate.max(1)),
        rate,
        natural: plan.slots.iter().filter(|s| s.fit == Fit::Natural).count(),
        adjusted: plan.slots.iter().filter(|s| s.fit == Fit::Adjusted).count(),
        overflowing: plan.slots.iter().filter(|s| s.fit == Fit::Overflow).count(),
        worst_over_s: plan.worst_over_s,
    })
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
///
/// # Errors
///
/// When the id names a model that is not a voice, a voice that does not speak `lang`, or nothing at
/// all — each naming the installed voices that would have worked.
pub fn resolve_voice(paths: &Paths, wanted: Option<&str>, lang: &str) -> Result<PathBuf> {
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

/// Whether any installed voice could dub into `lang`.
///
/// Asked by the meeting screen before it offers the button. A control that is offered and then
/// refuses is worse than one that explains why it is disabled.
#[must_use]
pub fn can_speak(paths: &Paths, lang: &str) -> bool {
    let store = summo_models::ModelStore::new(paths.clone());
    installed_voices(&store)
        .iter()
        .any(|m| summo_models::langs_cover(&m.langs, lang))
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

/// The original recording, to sit under the dub. Absent is fine — the dub stands alone.
///
/// A previous dub is never the bed. Without that filter the second language dubbed over a meeting
/// gets the first one underneath it, two voices at once, and the third gets both.
fn load_under(paths: &Paths, id: &MeetingId, rate: u32) -> Vec<f32> {
    let dir = paths.audio_for(id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let Some(wav) = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dub-"))
        })
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

    /// The language arrives in a request body and ends up in a file name. Nothing that could leave
    /// the meeting's own directory may survive the trip.
    #[test]
    fn a_language_that_looks_like_a_path_does_not_stay_one() {
        assert_eq!(normalise_lang("../../etc/passwd"), "etcpasswd");
        assert_eq!(normalise_lang("vi/../.."), "vi");
        assert_eq!(normalise_lang("zh-Hans"), "zh-hans");
        assert_eq!(normalise_lang("  EN  "), "en");
        // Long enough to be a real tag, short enough not to be a payload.
        assert_eq!(normalise_lang(&"a".repeat(100)).len(), 16);
    }

    /// What this writes and what the player asks for are the same string, or the dub is invisible.
    #[test]
    fn the_lane_written_is_the_lane_the_player_can_fetch() {
        let lane = lane_name("vi");
        assert_eq!(lane, "dub-vi");
        assert!(
            crate::audio_stream::is_dub_lane(&lane),
            "`{lane}` is written by this module and not recognised by audio_stream"
        );
    }

    /// Half the bar per pass, so it fills once across work that happens twice.
    #[test]
    fn the_second_pass_starts_where_the_first_finished() {
        let first = JobState::Speaking {
            pass: 1,
            spoken: 10,
            total: 10,
        };
        let second = JobState::Speaking {
            pass: 2,
            spoken: 0,
            total: 10,
        };
        assert_eq!(first.fraction(), Some(0.5));
        assert_eq!(second.fraction(), Some(0.5));
        assert_eq!(
            JobState::Speaking {
                pass: 2,
                spoken: 10,
                total: 10
            }
            .fraction(),
            Some(1.0)
        );
    }

    #[test]
    fn a_queued_job_has_no_honest_fraction() {
        assert_eq!(JobState::Queued.fraction(), None);
        assert_eq!(JobState::Loading.fraction(), None);
        assert_eq!(JobState::Failed { error: "x".into() }.fraction(), None);
    }

    #[test]
    fn a_registry_lists_newest_first_and_clears_only_what_has_stopped() {
        let dubs = Dubs::new();
        let first = dubs.add("m1", "One", "vi");
        let second = dubs.add("m2", "Two", "en");
        assert_eq!(
            dubs.list().iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
            vec![second.clone(), first.clone()]
        );
        assert!(dubs.busy());

        dubs.set(
            &first,
            JobState::Failed {
                error: "no voice".into(),
            },
        );
        assert!(dubs.busy(), "the second job is still queued");
        assert_eq!(dubs.clear_finished(), 1);
        assert_eq!(dubs.list().len(), 1);
        assert_eq!(dubs.get(&second).unwrap().meeting, "m2");
    }
}
