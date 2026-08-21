//! What a new install is still missing, and what to do about it.
//!
//! Summo's first run is unusually demanding for a note-taking app: it needs a speech model before
//! it can do anything at all, and that model is hundreds of megabytes. An app that opens to an
//! empty screen and a `Record` button that fails is an app that gets deleted in the first minute.
//!
//! So the daemon answers one question — *what stands between this user and a working recording* —
//! and answers it as a list of steps with a state each, rather than as a wizard. The difference
//! matters: a wizard is a sequence somebody has to finish, and a user who quits halfway is stuck
//! outside it. This is a checklist that is recomputed from the machine every time it is asked, so
//! quitting and coming back picks up exactly where things stand, and a user who already has models
//! from a previous install sees them already ticked.
//!
//! Only [`Step::Models`] actually blocks recording. ffmpeg is needed for importing files, and a
//! language model for summaries — both real features, neither one required to take a note. Saying
//! so is what lets a user get to their first recording in one step instead of four.

use serde::Serialize;
use summo_core::{Result, paths::Paths};
use summo_models::{ModelStore, Task, hw::HwProfile};

/// Marks setup as acknowledged, so the checklist stops being the first thing on screen.
///
/// A file rather than a settings field: it is a fact about this installation's history, not a
/// preference, and it should not travel to another machine through a synced settings file.
const DONE_FILE: &str = "onboarded";

/// One thing that may or may not be ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// A speech recognition model. Nothing works without one.
    Models,
    /// ffmpeg, for importing files that are not already 16 kHz WAV.
    Ffmpeg,
    /// A language model, for summaries, translation and chat.
    Llm,
}

impl Step {
    /// Whether recording is impossible without it.
    #[must_use]
    pub fn blocking(self) -> bool {
        matches!(self, Step::Models)
    }
}

/// The state of one step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub step: Step,
    pub ready: bool,
    pub blocking: bool,
    /// What is there, or what is missing — shown next to the step.
    pub detail: String,
    /// Which pieces are absent, by name, for a client that has to *do* something about it.
    ///
    /// `detail` is a Vietnamese sentence written for a human. The setup screen needs to know
    /// whether to install a voice detector as well as a recogniser, and reading that out of a
    /// sentence is the kind of coupling that breaks the day somebody rewords it — and it did: the
    /// check for it matched on the words "dò giọng".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<&'static str>,
}

/// Everything a new install needs to know about itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// The user has been through setup at least once.
    pub acknowledged: bool,
    /// Nothing is blocking a recording.
    pub can_record: bool,
    /// No meetings on disk. A user with meetings is not a new user however the checklist looks.
    pub fresh: bool,
    pub checks: Vec<Check>,
    /// What this machine looks like to the model picker, so the wizard can explain its choice.
    pub hardware: HwProfile,
    /// Whether this build can transcribe at all.
    ///
    /// A property of the binary, not of the machine: recognition is a compile-time feature, and
    /// `--no-models` builds — the small tarball, and the Android emulator build, which cannot have
    /// it because ONNX Runtime publishes nothing for x86-64 Android — are missing every model the
    /// user might install.
    ///
    /// Reported because the app was happy to sell what it could not do. On Android, where the
    /// feature had never been enabled, setup offered the catalogue, downloaded 99 MB of Whisper,
    /// said "Ready to record" and then failed the recording with `session needs a live model`. The
    /// checklist knew about the file on disk and nothing about whether anything could read it.
    pub recognition: bool,
}

impl Status {
    /// Whether setup should take over the screen.
    ///
    /// Only for a genuinely new install. A user with meetings on disk and a deleted model directory
    /// has a *broken* install, not a new one — and hiding their notes behind a setup screen would
    /// take away the one thing that still works. They get [`Status::needs_attention`] instead,
    /// which is a banner.
    #[must_use]
    pub fn should_prompt(&self) -> bool {
        self.fresh && !self.acknowledged
    }

    /// Whether something is wrong that the user has to be told about, without being blocked.
    #[must_use]
    pub fn needs_attention(&self) -> bool {
        !self.can_record
    }
}

/// Look at the machine and report.
#[must_use]
pub fn status(paths: &Paths, hardware: &HwProfile) -> Status {
    let store = ModelStore::new(paths.clone());
    let installed = store.list();

    // What this binary can do, before what is on the disk. A build without recognition cannot use
    // a model however many are installed, and saying "ready" because a file is present is how an
    // app ends up promising a recording it will refuse.
    let recognition = cfg!(feature = "models");

    // Both halves, because recording needs both.
    //
    // This counted speech models only, so a vault with a recogniser and no voice detector was
    // "ready to record" — and then the pipeline refused at `resolve_vad`, the app kept its timer
    // running, and no words ever arrived. The detector is what decides where an utterance ends;
    // without it nothing is ever committed to a transcript, however good the recogniser is.
    let asr: Vec<_> = installed.iter().filter(|m| m.task == Task::Asr).collect();
    let vad: Vec<_> = installed.iter().filter(|m| m.task == Task::Vad).collect();
    // Not a blocker, and still worth reporting: without a voice fingerprint a recording runs and
    // cannot say who spoke, and the setup screen has no way to show that unless it is told.
    let speaker: Vec<_> = installed
        .iter()
        .filter(|m| m.task == Task::SpeakerEmbed)
        .collect();
    let models = Check {
        step: Step::Models,
        ready: recognition && !asr.is_empty() && !vad.is_empty(),
        blocking: Step::Models.blocking(),
        detail: match (recognition, asr.first(), vad.first()) {
            (false, _, _) => "bản dựng này không có nhận dạng giọng nói".into(),
            (true, Some(model), Some(_)) => model.id.to_string(),
            (true, Some(_), None) => "chưa cài bộ dò giọng nói".into(),
            (true, None, Some(_)) => "chưa cài mô hình nhận dạng nào".into(),
            (true, None, None) => "chưa cài mô hình nhận dạng và bộ dò giọng nói".into(),
        },
        missing: [
            asr.is_empty().then_some("asr"),
            vad.is_empty().then_some("vad"),
            speaker.is_empty().then_some("speaker"),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };

    // Ready either way, because importing no longer depends on it.
    //
    // Summo decodes mp3, m4a, mp4, mov, mkv, wav, flac and Vorbis in this process. ffmpeg adds the
    // remainder — Opus in WebM, AC-3, WMA — so its absence is a smaller catalogue of formats, not
    // a broken feature, and a setup checklist that tells somebody to go and install a program
    // before their app will work is the opposite of what a download should be.
    let ffmpeg = match summo_media::probe() {
        Ok(tools) => Check {
            step: Step::Ffmpeg,
            ready: true,
            blocking: false,
            detail: tools.version,
            missing: Vec::new(),
        },
        Err(_) => Check {
            step: Step::Ffmpeg,
            ready: true,
            blocking: false,
            detail: "built-in".into(),
            missing: Vec::new(),
        },
    };

    let llm = llm_check(paths);

    let checks = vec![models, ffmpeg, llm];
    let can_record = checks.iter().all(|c| !c.blocking || c.ready);

    Status {
        acknowledged: acknowledged(paths),
        can_record,
        fresh: is_fresh(paths),
        checks,
        hardware: hardware.clone(),
        recognition,
    }
}

/// Whether a language model is configured *and* usable.
///
/// Configured is not enough: choosing OpenAI without setting `SUMMO_API_KEY` is the most common way
/// to end up with a provider that is set and silently does nothing. The key deliberately lives in
/// the environment rather than in the settings file — see `summo_core::settings::Llm` — so this is
/// the only place that can tell.
fn llm_check(paths: &Paths) -> Check {
    let Ok(settings) = summo_core::settings::Settings::load(&paths.settings()) else {
        return Check {
            step: Step::Llm,
            ready: false,
            blocking: false,
            detail: "chưa đọc được cài đặt".into(),
            missing: Vec::new(),
        };
    };

    let provider = settings.llm.provider.trim();
    if provider.is_empty() {
        return Check {
            step: Step::Llm,
            ready: false,
            blocking: false,
            detail: "chưa chọn mô hình ngôn ngữ".into(),
            missing: Vec::new(),
        };
    }

    let hosted = matches!(provider, "openai" | "anthropic");
    let key = std::env::var("SUMMO_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty());

    if hosted && key.is_none() {
        return Check {
            step: Step::Llm,
            ready: false,
            blocking: false,
            detail: format!("{provider} cần SUMMO_API_KEY"),
            missing: Vec::new(),
        };
    }

    Check {
        step: Step::Llm,
        ready: true,
        blocking: false,
        detail: provider.to_string(),
        missing: Vec::new(),
    }
}

/// Whether the vault has anything in it at all.
///
/// Notes count, not just meetings. A user who typed three notes before installing a model has an
/// install with work in it, and greeting them with a welcome screen every launch reads as an app
/// that has not noticed they are already using it.
fn is_fresh(paths: &Paths) -> bool {
    !has_markdown(&paths.meetings()) && !has_markdown(&paths.notes())
}

fn has_markdown(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
}

#[must_use]
pub fn acknowledged(paths: &Paths) -> bool {
    paths.root().join(DONE_FILE).exists()
}

/// Record that the user has seen setup.
pub fn acknowledge(paths: &Paths) -> Result<()> {
    let path = paths.root().join(DONE_FILE);
    std::fs::create_dir_all(paths.root()).map_err(|e| summo_core::Error::io(paths.root(), e))?;
    std::fs::write(&path, b"").map_err(|e| summo_core::Error::io(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw() -> HwProfile {
        HwProfile::detect()
    }

    #[test]
    fn a_machine_with_no_models_cannot_record_and_says_which_step_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let status = status(&Paths::at(tmp.path()), &hw());

        assert!(!status.can_record);
        let models = status
            .checks
            .iter()
            .find(|c| c.step == Step::Models)
            .expect("models check");
        assert!(!models.ready);
        assert!(models.blocking);
    }

    /// A manifest on disk is what `ModelStore::list` reads, so this is what "installed" means here.
    fn install(paths: &Paths, id: &str, task: &str) {
        let dir = paths.manifests();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::json!({
                "schema": 1,
                "id": id,
                "name": id,
                "task": task,
                "mode": "live",
                "runtime": "test/runtime",
                "license": "MIT",
                "files": [{
                    "name": "model.onnx",
                    "sha256": "c".repeat(64),
                    "size": 1024,
                    "url": "https://example.invalid/model.onnx"
                }]
            })
            .to_string(),
        )
        .unwrap();
    }

    /// The one that shipped: a recogniser and no voice detector reported "ready to record".
    ///
    /// The pipeline then refused at `resolve_vad`, the app kept a timer running, and no words ever
    /// arrived. A checklist that says ready has to mean it — this is the assertion that makes the
    /// two halves agree.
    #[test]
    fn a_recogniser_without_a_voice_detector_is_not_ready_to_record() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        install(&paths, "gipformer-65m", "asr");

        let status = super::status(&paths, &hw());
        let models = status
            .checks
            .iter()
            .find(|c| c.step == Step::Models)
            .expect("models check");

        if cfg!(feature = "models") {
            assert!(
                !models.ready,
                "a vault with no voice detector said it was ready"
            );
            assert!(
                models.detail.contains("dò giọng"),
                "the missing half is not named: {}",
                models.detail
            );
            assert!(!status.can_record);

            install(&paths, "silero-vad-v5", "vad");
            let with_both = super::status(&paths, &hw());
            let models = with_both
                .checks
                .iter()
                .find(|c| c.step == Step::Models)
                .expect("models check");
            assert!(
                models.ready,
                "both installed and still not ready: {}",
                models.detail
            );
        }
    }

    /// The point of separating blocking from ready: no ffmpeg means no *importing*, and a user who
    /// only wants to record a meeting should not be stopped by it.
    #[test]
    fn a_missing_ffmpeg_does_not_block_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let status = status(&Paths::at(tmp.path()), &hw());
        let ffmpeg = status
            .checks
            .iter()
            .find(|c| c.step == Step::Ffmpeg)
            .unwrap();
        assert!(!ffmpeg.blocking);
    }

    /// Choosing OpenAI and never setting a key is the most common way to end up with a provider
    /// that looks configured and silently does nothing.
    #[test]
    fn a_hosted_provider_without_a_key_is_not_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();

        let mut settings = summo_core::settings::Settings::default();
        settings.llm.provider = "openai".into();
        settings.save(&paths.settings()).unwrap();

        // SAFETY: single-threaded test; the variable is removed, not read concurrently.
        unsafe { std::env::remove_var("SUMMO_API_KEY") };

        let llm = llm_check(&paths);
        assert!(!llm.ready);
        assert!(llm.detail.contains("SUMMO_API_KEY"), "{}", llm.detail);
    }

    #[test]
    fn a_local_provider_needs_no_key() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();

        let mut settings = summo_core::settings::Settings::default();
        settings.llm.provider = "ollama".into();
        settings.save(&paths.settings()).unwrap();

        assert!(llm_check(&paths).ready);
    }

    #[test]
    fn acknowledging_survives_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        assert!(!acknowledged(&paths));
        acknowledge(&paths).unwrap();
        assert!(acknowledged(&paths));
    }

    /// A user with notes and a deleted model directory has a broken install, not a new one. Hiding
    /// their notes behind a setup screen would take away the one thing that still works — so they
    /// get told, not blocked.
    #[test]
    fn a_broken_install_warns_without_hiding_the_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(paths.meetings().join("hop.md"), "# Họp").unwrap();
        acknowledge(&paths).unwrap();

        let status = status(&paths, &hw());
        assert!(status.needs_attention(), "no models: has to say so");
        assert!(!status.should_prompt(), "but must not hide the notes");
    }

    /// The case setup exists for.
    #[test]
    fn a_brand_new_install_leads_with_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let status = status(&Paths::at(tmp.path()), &hw());
        assert!(status.should_prompt());
    }

    #[test]
    fn a_vault_with_meetings_in_it_is_not_a_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(paths.meetings().join("hop.md"), "# Họp").unwrap();

        assert!(!status(&paths, &hw()).fresh);
    }

    /// A user who typed a few notes before installing a model is already using the app; greeting
    /// them every launch reads as an app that has not noticed.
    #[test]
    fn a_vault_with_only_notes_is_not_a_fresh_install_either() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.notes()).unwrap();
        std::fs::write(paths.notes().join("y-tuong.md"), "# Ý tưởng").unwrap();

        assert!(!status(&paths, &hw()).fresh);
    }

    #[test]
    fn an_empty_vault_is_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        assert!(status(&paths, &hw()).fresh);
    }

    /// A working install that has been acknowledged should not keep showing a welcome screen.
    #[test]
    fn a_ready_and_acknowledged_install_does_not_prompt() {
        let status = Status {
            acknowledged: true,
            can_record: true,
            fresh: true,
            checks: Vec::new(),
            hardware: hw(),
            recognition: true,
        };
        assert!(!status.should_prompt());
    }

    /// Someone restoring a vault onto a new machine has meetings and no acknowledgement; they do
    /// not need a welcome tour, they need their notes.
    #[test]
    fn an_existing_vault_does_not_prompt_just_because_the_flag_is_missing() {
        let status = Status {
            acknowledged: false,
            can_record: true,
            fresh: false,
            checks: Vec::new(),
            hardware: hw(),
            recognition: true,
        };
        assert!(!status.should_prompt());
    }

    /// A build with no recognition in it must not say a recording can start.
    ///
    /// This is the Android bug as a test. `models` was not enabled there, so the store had a model
    /// in it, the checklist called that ready, and every recording then failed with `session needs
    /// a live model`. `cargo test` runs without the feature, which is exactly the build being
    /// described.
    #[cfg(not(feature = "models"))]
    #[test]
    fn a_build_that_cannot_transcribe_does_not_claim_a_recording_can_start() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();

        let status = status(&paths, &hw());
        assert!(!status.recognition, "a build without the feature claims it");
        assert!(!status.can_record, "it offered to record anyway");
        let models = status
            .checks
            .iter()
            .find(|c| c.step == Step::Models)
            .expect("the models step");
        assert!(!models.ready);
        assert!(
            models.detail.contains("nhận dạng"),
            "the reason has to name recognition, not a missing file: {}",
            models.detail
        );
    }
}
