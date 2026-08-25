//! What the user has chosen.
//!
//! One JSON file, read by the daemon, the CLI and the interface alike, so there is a single answer
//! to "which model is this using" rather than three that can disagree.
//!
//! Two rules shape the type. Every field has a default, so a settings file written by an older
//! build still loads and a missing file is not an error — the app has to start on a machine that
//! has never run it. And unknown fields are preserved rather than dropped, so a newer build's
//! settings survive a round trip through an older one instead of being silently erased the first
//! time somebody downgrades.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Everything the user can change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    pub recording: Recording,
    pub models: Models,
    pub llm: Llm,
    pub storage: Storage,
    pub interface: Interface,
    pub agents: Agents,
    /// Fields this build does not know about, kept so a downgrade does not erase them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// What the agents do when nobody is asking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Agents {
    /// Once a day, let each agent re-read and shorten its own memory.
    ///
    /// Off by default. It costs a language-model call per agent per day, and on the first day
    /// there is nothing to consolidate — a memory of four lines is already as short as it gets.
    pub dream: bool,
    /// Hour, local, after which it may happen. Late, because it is unattended work on a file that
    /// steers every later answer, and the user should be asleep rather than mid-sentence.
    pub dream_hour: u8,
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recording {
    /// Capture the system audio as well as the microphone.
    pub capture_system_audio: bool,
    /// Input device id, or `None` to pick the best one.
    pub device_id: Option<String>,
    /// Global shortcut that starts and stops recording.
    pub hotkey: String,
    /// Suggest recording when a call application is running. Never records on its own.
    pub suggest_on_meeting: bool,
    /// Speech probability above which a frame counts as speech.
    pub vad_threshold: f32,
    /// Trailing silence before an utterance is committed, milliseconds.
    ///
    /// The single most felt setting: it is added directly to the delay before final text appears,
    /// and cutting it too far truncates sentences.
    pub min_silence_ms: u32,
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// Every field is `None` until the user or `summo setup` chooses, so a fresh install has no
/// opinion rather than a wrong one baked in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Models {
    /// Model driving live text.
    pub live: Option<String>,
    /// Slower model that re-decodes finished utterances.
    pub refine: Option<String>,
    pub vad: Option<String>,
    pub speaker: Option<String>,
    /// Speech enhancer, run over each finished utterance before it is decoded.
    ///
    /// `None` is the default and the right answer for most rooms — see `summo_asr::denoise` on why
    /// turning this on for everybody would improve the noisy meetings and quietly damage the quiet
    /// ones.
    pub denoise: Option<String>,
    /// Voice for `summo dub`.
    ///
    /// A voice became installable one release before it became choosable, so for that release the
    /// only way to use one was to type its id on a command line — which is the state publishing it
    /// was supposed to end. `None` means "the one installed voice", which is the answer on every
    /// machine with exactly one; it only becomes a real choice when there are two.
    pub tts: Option<String>,
    /// ISO code, or `None` to let the model detect.
    pub language: Option<String>,
    /// Threads for inference. `None` follows the hardware probe.
    pub threads: Option<usize>,
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// Where summaries, translation and answers come from.
///
/// Distinct from the model registry, which is Ollama-*shaped* — `pull`, `list`, content-addressed
/// blobs — but has nothing to do with Ollama the program. Speech models are Summo's own; this
/// setting is the separate question of which language model the user points the text features at,
/// and Ollama is merely one common answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Llm {
    /// `ollama`, `lm-studio`, `openai`, `anthropic`, or a base URL for anything OpenAI-compatible.
    pub provider: String,
    pub model: Option<String>,
    /// Language summaries and answers are written in.
    pub language: String,
    /// Summarise automatically when a recording stops.
    pub summarize_on_stop: bool,
    /// A second, smaller model that does translation only. `None` means translation goes to the
    /// same model as everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translator: Option<Translator>,
    // No API key. Keys live in the OS keychain; one in a settings file would end up in backups,
    // sync and support bundles.
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// A dedicated machine-translation model.
///
/// Separate from [`Llm`] because summarising and translating want different models, and pretending
/// otherwise costs either money or quality. A summary needs a model that can read a messy
/// transcript and reason about it. A translation needs a model that has seen a great deal of
/// parallel text — and a *1B* model that has is better at translating than a general 8B one, runs
/// on a CPU in under a second a line, and costs nothing per line forever.
///
/// That is the whole argument for the field existing: it is what lets the expensive model be
/// optional. A user with no LLM key at all can still translate every meeting they record.
///
/// Points at an OpenAI-compatible endpoint like everything else — llama.cpp, Ollama, LM Studio —
/// so this adds a setting, not a runtime. What it changes is the *prompt*: see
/// [`summo_llm::prompt::mt`], which is the string these models were trained to continue and which
/// is not interchangeable with the instruction-following one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Translator {
    /// Where the model runs.
    ///
    /// `local` loads it into the daemon and needs nothing else installed. Anything else is a
    /// preset id or a base URL, exactly as [`Llm::provider`] — for somebody who already runs
    /// Ollama and would rather keep one model server than two.
    pub provider: String,
    /// Which model: a registry id such as `milmmt-46-1b` when `provider` is `local`, or the name
    /// the endpoint knows it by otherwise.
    pub model: Option<String>,
}

impl Translator {
    /// Whether this translator runs inside the daemon.
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.provider.trim().eq_ignore_ascii_case(LOCAL)
    }
}

/// The provider name that means "in this process".
///
/// Not a URL and never resolved against the endpoint catalogue: it is the absence of an endpoint.
pub const LOCAL: &str = "local";

impl Default for Translator {
    fn default() -> Self {
        Self {
            // In this process, which is the whole point: translation is the one text feature that
            // can cost nothing, and it only actually does if there is nothing to install first.
            provider: LOCAL.into(),
            // The small one. A default that is the best model rather than the one that fits is a
            // default nobody can use — 611 MB installs on a laptop, and somebody who wants the
            // more accurate 806 MB model can say so.
            model: Some("small100".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Storage {
    /// Delete recordings after this many days, keeping the transcript. Zero keeps them forever.
    pub audio_retention_days: u32,
    /// Keep the audio alongside the transcript at all.
    pub keep_audio: bool,
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// What the app looks like, and in which language.
///
/// Here rather than only in the browser's `localStorage` for the reason `models.language` is: a
/// choice that lives in one browser is a choice a second window, a second machine and the tray have
/// never heard of. The browser still decides what to paint *now* — reading this before first paint
/// would add a round trip in front of the first pixel — but it adopts this when it has no choice of
/// its own, and writes back when the user makes one.
///
/// `compact_while_recording` and `show_performance` used to be here. Both were dead: nothing in the
/// daemon, the app or the shell ever read either one, and they named features that were never
/// built — shrinking is a button somebody presses, and there is no performance readout to toggle.
/// A field a user can set in `settings.json` that changes nothing is worse than no field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// The interface language as a tag — `vi`, `en`, `ja`, `zh` — or empty to follow the browser.
    ///
    /// Empty is not the same as a language: it means nobody has chosen, and the app should keep
    /// asking the browser. Storing today's answer instead would freeze a machine that later changes
    /// its own locale.
    pub language: String,
    /// Fields a newer build wrote that this one does not know, kept so a downgrade does not erase
    /// them. See {@link Settings::unknown} — the guarantee has to hold at every level, because the
    /// place a new setting appears is almost always inside one of these, not beside them.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

impl Default for Agents {
    fn default() -> Self {
        Self {
            dream: false,
            dream_hour: 3,
            unknown: BTreeMap::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            recording: Recording::default(),
            models: Models::default(),
            llm: Llm::default(),
            storage: Storage::default(),
            interface: Interface::default(),
            agents: Agents::default(),
            unknown: BTreeMap::new(),
        }
    }
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            capture_system_audio: true,
            device_id: None,
            hotkey: "CmdOrCtrl+Shift+R".into(),
            suggest_on_meeting: true,
            // The production preset, measured on laptop microphones with fan noise present.
            vad_threshold: 0.35,
            min_silence_ms: 400,
            unknown: BTreeMap::new(),
        }
    }
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            // Local by default: the suggestion that keeps everything on the machine.
            provider: "ollama".into(),
            model: None,
            language: "Vietnamese".into(),
            summarize_on_stop: true,
            // Nothing by default: a setting that names an endpoint nobody is running turns every
            // translation into a connection error, which is worse than translating with the model
            // the user did configure.
            translator: None,
            unknown: BTreeMap::new(),
        }
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            audio_retention_days: 30,
            keep_audio: true,
            unknown: BTreeMap::new(),
        }
    }
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            // Empty, not "vi". A default language here would speak for a user who has not chosen
            // one, and it would speak louder than their browser — a fresh install on a Japanese
            // machine would open in Vietnamese because a struct had an opinion. Nobody has chosen
            // is a state, and the app already knows what to do with it: ask the browser.
            language: String::new(),
            unknown: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Read settings, falling back to defaults when the file does not exist.
    ///
    /// A corrupt file is an error rather than a silent reset: quietly replacing someone's
    /// configuration with defaults is worse than refusing to start and saying why.
    pub fn load(path: &Path) -> Result<Self> {
        let Ok(body) = std::fs::read_to_string(path) else {
            return Ok(Self::default());
        };
        let settings: Self = serde_json::from_str(&body)
            .map_err(|e| Error::Config(format!("{} is not valid settings: {e}", path.display())))?;

        if settings.schema > 1 {
            return Err(Error::Config(format!(
                "{} was written by a newer version of Summo (schema {})",
                path.display(),
                settings.schema
            )));
        }
        Ok(settings.clamped())
    }

    /// Write through a temporary file and rename, so an interrupted save cannot leave settings
    /// truncated and unreadable on next launch.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let temporary = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self)?;
        std::fs::write(&temporary, body).map_err(|e| Error::io(&temporary, e))?;
        std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))?;
        Ok(())
    }

    /// Bring hand-edited values back into ranges the pipeline can actually use.
    ///
    /// The file is meant to be editable, so it will be edited by hand and by scripts. Refusing to
    /// start over a typo would be worse than correcting it; refusing to *correct* it would mean a
    /// threshold of 9.0 silently disables voice detection.
    #[must_use]
    pub fn clamped(mut self) -> Self {
        self.recording.vad_threshold = self.recording.vad_threshold.clamp(0.05, 0.95);
        self.recording.min_silence_ms = self.recording.min_silence_ms.clamp(100, 5_000);
        self.models.threads = self.models.threads.map(|t| t.clamp(1, 32));
        if !matches!(self.interface.theme.as_str(), "system" | "light" | "dark") {
            self.interface.theme = "system".into();
        }
        self
    }

    /// Read one value by dotted path, for `summo config get`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        let value = serde_json::to_value(self).ok()?;
        let mut current = &value;
        for part in key.split('.') {
            current = current.get(part)?;
        }
        Some(current.clone())
    }

    /// Set one value by dotted path, for `summo config set`.
    ///
    /// Rejects an unknown key rather than storing it. A typo that silently writes
    /// `recording.vad_treshold` would leave the user adjusting a setting nothing reads.
    pub fn set(&mut self, key: &str, raw: &str) -> Result<()> {
        if self.get(key).is_none() {
            return Err(Error::Config(format!("no such setting: {key}")));
        }

        // Accept JSON where it parses, so `true`, `0.5` and `null` mean what they look like; fall
        // back to a string for everything else, so a hotkey does not need quoting.
        let parsed: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.into()));

        let mut value = serde_json::to_value(&*self)?;
        let mut current = &mut value;
        let parts: Vec<&str> = key.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            current = current
                .get_mut(part)
                .ok_or_else(|| Error::Config(format!("no such setting: {key}")))?;
        }
        let last = parts[parts.len() - 1];
        current
            .as_object_mut()
            .ok_or_else(|| Error::Config(format!("{key} is not a settings group")))?
            .insert(last.to_string(), parsed);

        let updated: Self = serde_json::from_value(value)
            .map_err(|e| Error::Config(format!("{key} does not accept that value: {e}")))?;
        *self = updated.clamped();
        Ok(())
    }

    /// Every settable key, for `summo config list` and the settings screen.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(serde_json::Value::Object(groups)) = serde_json::to_value(self) {
            for (group, value) in groups {
                match value {
                    serde_json::Value::Object(fields) => {
                        for field in fields.keys() {
                            out.push(format!("{group}.{field}"));
                        }
                    }
                    _ => out.push(group),
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_yields_defaults_rather_than_an_error() {
        // The app has to start on a machine that has never run it.
        let settings = Settings::load(Path::new("/nonexistent/settings.json")).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(
            settings.llm.provider, "ollama",
            "the default keeps data local"
        );
    }

    #[test]
    fn a_corrupt_file_is_refused_rather_than_silently_reset() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();

        let err = Settings::load(&path).unwrap_err().to_string();
        assert!(err.contains("not valid settings"), "got: {err}");
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_misread() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, r#"{"schema": 99}"#).unwrap();
        assert!(Settings::load(&path).is_err());
    }

    #[test]
    fn settings_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");

        let mut settings = Settings::default();
        settings.models.live = Some("gipformer-65m".into());
        settings.recording.min_silence_ms = 250;
        settings.save(&path).unwrap();

        assert_eq!(Settings::load(&path).unwrap(), settings);
    }

    #[test]
    fn a_newer_builds_settings_survive_an_older_one() {
        // Otherwise downgrading once erases whatever the newer version was configured with.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"schema":1,"future_feature":{"enabled":true},"llm":{"provider":"ollama"}}"#,
        )
        .unwrap();

        let settings = Settings::load(&path).unwrap();
        settings.save(&path).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("future_feature"),
            "an unknown field was dropped: {body}"
        );
    }

    #[test]
    fn hand_edited_values_are_brought_back_into_range() {
        // The file is meant to be editable, so it will be edited badly. A threshold of 9.0 would
        // otherwise disable voice detection entirely and look like a broken microphone.
        let mut settings = Settings::default();
        settings.recording.vad_threshold = 9.0;
        settings.recording.min_silence_ms = 1;
        settings.interface.theme = "neon".into();

        let settings = settings.clamped();
        assert!(settings.recording.vad_threshold <= 0.95);
        assert_eq!(settings.recording.min_silence_ms, 100);
        assert_eq!(settings.interface.theme, "system");
    }

    #[test]
    fn values_are_readable_and_writable_by_dotted_path() {
        let mut settings = Settings::default();

        settings.set("models.live", "gipformer-65m").unwrap();
        assert_eq!(settings.get("models.live").unwrap(), "gipformer-65m");

        settings
            .set("recording.capture_system_audio", "false")
            .unwrap();
        assert!(!settings.recording.capture_system_audio);

        settings.set("recording.min_silence_ms", "250").unwrap();
        assert_eq!(settings.recording.min_silence_ms, 250);
    }

    #[test]
    fn a_hotkey_does_not_need_quoting() {
        let mut settings = Settings::default();
        settings
            .set("recording.hotkey", "CmdOrCtrl+Shift+S")
            .unwrap();
        assert_eq!(settings.recording.hotkey, "CmdOrCtrl+Shift+S");
    }

    #[test]
    fn a_mistyped_key_is_refused_rather_than_stored() {
        // Storing it would leave the user adjusting a setting nothing ever reads.
        let mut settings = Settings::default();
        let err = settings.set("recording.vad_treshold", "0.5").unwrap_err();
        assert!(err.to_string().contains("no such setting"), "got: {err}");
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused() {
        let mut settings = Settings::default();
        assert!(
            settings
                .set("recording.min_silence_ms", "\"soon\"")
                .is_err()
        );
    }

    #[test]
    fn setting_a_value_out_of_range_clamps_it() {
        let mut settings = Settings::default();
        settings.set("recording.vad_threshold", "9").unwrap();
        assert!(settings.recording.vad_threshold <= 0.95);
    }

    #[test]
    fn no_api_key_can_be_stored_in_settings() {
        // Keys live in the OS keychain; one here would reach backups, sync and support bundles.
        let settings = Settings::default();
        assert!(!settings.keys().iter().any(|k| k.contains("api_key")));

        let json = serde_json::to_string(&settings).unwrap();
        assert!(
            !json.contains("api_key"),
            "settings gained a key field: {json}"
        );
    }

    #[test]
    fn every_group_is_listed_for_the_settings_screen() {
        let keys = Settings::default().keys();
        for expected in [
            "recording.hotkey",
            "models.live",
            "llm.provider",
            "storage.audio_retention_days",
            "interface.theme",
        ] {
            assert!(
                keys.contains(&expected.to_string()),
                "missing {expected} in {keys:?}"
            );
        }
    }
}

#[cfg(test)]
mod downgrade {
    use super::*;

    /// A setting this build has never heard of survives being read and written back.
    ///
    /// `Settings` has carried an `unknown` catch-all since it had a schema, with a comment saying
    /// why: somebody who runs a newer build, then an older one, must not have their configuration
    /// quietly emptied by the older one saving over it. The guarantee held at the top level and
    /// nowhere else — and the top level is the one place new settings almost never appear. They
    /// appear inside `models`, `llm`, `recording`, which had no catch-all at all.
    ///
    /// Measured before the fix: `{"tomorrow": …}` survived, `{"models": {"tomorrow": …}}` did not.
    /// So a user on a newer build who pinned a model with an option this build lacks, opened the
    /// older one once, and touched any setting, lost it.
    #[test]
    fn a_field_from_a_newer_build_survives_an_older_one() {
        let written = r#"{
            "tomorrow": "beside",
            "models": { "live": "whisper-tiny", "tomorrow": "inside" },
            "llm": { "provider": "ollama", "tomorrow": 7 },
            "recording": { "tomorrow": true },
            "storage": { "tomorrow": [1] },
            "interface": { "theme": "dark", "tomorrow": null },
            "agents": { "tomorrow": {"a": 1} }
        }"#;

        let settings: Settings = serde_json::from_str(written).expect("parses");
        let back = serde_json::to_string(&settings).expect("serialises");

        // The settings this build *does* know are still themselves.
        assert_eq!(settings.models.live.as_deref(), Some("whisper-tiny"));
        assert_eq!(settings.interface.theme, "dark");

        // And every stranger is still there, at whichever level it was found.
        for (place, section) in [
            ("beside", &settings.unknown),
            ("inside", &settings.models.unknown),
        ] {
            assert_eq!(
                section.get("tomorrow").and_then(|v| v.as_str()),
                Some(place),
                "a field this build does not know was dropped"
            );
        }
        for section in [
            &settings.llm.unknown,
            &settings.recording.unknown,
            &settings.storage.unknown,
            &settings.interface.unknown,
            &settings.agents.unknown,
        ] {
            assert!(section.contains_key("tomorrow"), "dropped from a section");
        }
        assert_eq!(
            back.matches("tomorrow").count(),
            7,
            "all seven written back"
        );
    }
}
