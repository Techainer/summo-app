//! A meeting in another language, stored beside it.
//!
//! A translation is not a rewrite of the meeting — the original transcript stays the record of what
//! was actually said. So it lives in its own file, one per language, and the meeting note is
//! untouched. That also means deleting a translation is deleting a file, and translating twice into
//! two languages does not make the note twice as long.
//!
//! The file is Markdown, per ADR 0002, and each line carries the `seq` of the utterance it
//! translates. Aligning on `seq` rather than on position is what makes the file survive editing:
//! correcting a typo in the transcript, or splitting one segment into two, would silently shift
//! every subsequent line of a position-aligned file and attach every translation to the wrong
//! sentence.
//!
//! ```text
//! <!-- summo:translation lang:en model:gpt-4o-mini -->
//! [00:12] Hello everyone, thanks for joining. <!-- seq:3 -->
//! [00:19] Let's start with the budget. <!-- seq:4 -->
//! ```

use std::path::{Path, PathBuf};

use summo_core::{Error, MeetingId, Result, paths::Paths};

use crate::meeting::{format_timestamp, parse_timestamp};

/// Marks the file as machine-written, so a human editing it knows what they are looking at.
const HEADER: &str = "<!-- summo:translation";

/// One translated utterance.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The `seq` of the segment in the original transcript.
    pub seq: u64,
    /// Roughly when it was said, to the second.
    ///
    /// A reading aid, not a timing source. It is written as `[MM:SS]` so the file is legible on its
    /// own, which means it comes back rounded. Anything that needs real timing — subtitles, dubbing
    /// — looks the segment up by [`Line::seq`] and takes `t0`/`t1` from the transcript, where they
    /// are exact.
    pub t0: f64,
    pub text: String,
}

/// A whole meeting in one target language.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// A BCP-47-ish tag: `en`, `ja`, `vi`. Used in the filename, so it must not contain a
    /// separator — [`sanitize_lang`] enforces that.
    pub lang: String,
    /// The model that produced it, so a bad translation can be traced to a bad model.
    pub model: Option<String>,
    pub lines: Vec<Line>,
}

impl Translation {
    #[must_use]
    pub fn new(lang: impl Into<String>) -> Self {
        Self {
            lang: lang.into(),
            model: None,
            lines: Vec::new(),
        }
    }

    /// The translation of one segment, if it has one.
    #[must_use]
    pub fn get(&self, seq: u64) -> Option<&str> {
        self.lines
            .iter()
            .find(|l| l.seq == seq)
            .map(|l| l.text.as_str())
    }

    /// Add or replace one line, keeping the file ordered by time.
    ///
    /// Re-translating a single corrected sentence has to update in place rather than append, or the
    /// file accumulates two translations of the same utterance and `get` returns whichever came
    /// first.
    pub fn set(&mut self, seq: u64, t0: f64, text: impl Into<String>) {
        let text = text.into();
        match self.lines.iter_mut().find(|l| l.seq == seq) {
            Some(line) => {
                line.t0 = t0;
                line.text = text;
            }
            None => {
                self.lines.push(Line { seq, t0, text });
                self.lines
                    .sort_by(|a, b| a.t0.total_cmp(&b.t0).then(a.seq.cmp(&b.seq)));
            }
        }
    }

    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::from(HEADER);
        out.push_str(&format!(" lang:{}", self.lang));
        if let Some(model) = &self.model {
            out.push_str(&format!(" model:{model}"));
        }
        out.push_str(" -->\n\n");

        for line in &self.lines {
            out.push_str(&format!(
                "[{}] {} <!-- seq:{} -->\n",
                format_timestamp(line.t0),
                line.text,
                line.seq
            ));
        }
        out
    }

    pub fn parse(markdown: &str) -> Result<Self> {
        let mut translation = Translation::new("");

        for raw in markdown.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(HEADER) {
                for field in rest.trim_end_matches("-->").split_whitespace() {
                    match field.split_once(':') {
                        Some(("lang", value)) => translation.lang = value.to_string(),
                        Some(("model", value)) => translation.model = Some(value.to_string()),
                        _ => {}
                    }
                }
                continue;
            }

            // A line without a `seq` cannot be attached to anything, so it is a note somebody
            // added by hand and is left alone rather than guessed at.
            let Some(seq) = seq_of(line) else { continue };
            let Some(rest) = line.strip_prefix('[') else {
                continue;
            };
            let Some((stamp, text)) = rest.split_once(']') else {
                continue;
            };
            let t0 = parse_timestamp(stamp).unwrap_or(0.0);
            let text = text
                .split("<!--")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            translation.lines.push(Line { seq, t0, text });
        }

        if translation.lang.is_empty() {
            return Err(Error::Other(
                "bản dịch không ghi ngôn ngữ; thiếu dòng <!-- summo:translation lang:… -->".into(),
            ));
        }
        Ok(translation)
    }
}

fn seq_of(line: &str) -> Option<u64> {
    let start = line.find("<!-- seq:")? + "<!-- seq:".len();
    let rest = &line[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Where translations live.
#[must_use]
pub fn dir(paths: &Paths) -> PathBuf {
    paths.vault().join("translations")
}

/// A language tag safe to put in a filename.
///
/// The tag reaches this from an HTTP body, and a caller passing `../../settings` would otherwise
/// write outside the vault. Anything that is not a letter, digit or hyphen is dropped rather than
/// replaced, so `en-US` survives and `../x` becomes `x`.
#[must_use]
pub fn sanitize_lang(lang: &str) -> String {
    lang.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The file one meeting's translation into one language lives in.
pub fn path(paths: &Paths, meeting: &MeetingId, lang: &str) -> Result<PathBuf> {
    let lang = sanitize_lang(lang);
    if lang.is_empty() {
        return Err(Error::Other(format!("`{lang}` không phải mã ngôn ngữ")));
    }
    Ok(dir(paths).join(format!("{meeting}.{lang}.md")))
}

pub fn load(paths: &Paths, meeting: &MeetingId, lang: &str) -> Result<Option<Translation>> {
    let file = path(paths, meeting, lang)?;
    match std::fs::read_to_string(&file) {
        Ok(text) => Translation::parse(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&file, e)),
    }
}

pub fn save(paths: &Paths, meeting: &MeetingId, translation: &Translation) -> Result<PathBuf> {
    let file = path(paths, meeting, &translation.lang)?;
    let parent = dir(paths);
    std::fs::create_dir_all(&parent).map_err(|e| Error::io(&parent, e))?;
    crate::write::write_atomically(&file, translation.to_markdown().as_bytes())?;
    Ok(file)
}

/// Which languages a meeting has been translated into.
#[must_use]
pub fn languages(paths: &Paths, meeting: &MeetingId) -> Vec<String> {
    let prefix = format!("{meeting}.");
    let Ok(entries) = std::fs::read_dir(dir(paths)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?.strip_suffix(".md")?;
            (!rest.is_empty()).then(|| rest.to_string())
        })
        .collect();
    out.sort();
    out
}

/// Remove one translation. `false` when there was nothing there.
pub fn remove(paths: &Paths, meeting: &MeetingId, lang: &str) -> Result<bool> {
    let file = path(paths, meeting, lang)?;
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::io(&file, e)),
    }
}

/// Read a translation file straight from a path, for tooling that already has one.
pub fn read(file: &Path) -> Result<Translation> {
    let text = std::fs::read_to_string(file).map_err(|e| Error::io(file, e))?;
    Translation::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Translation {
        let mut t = Translation::new("en");
        t.model = Some("gpt-4o-mini".into());
        t.set(3, 12.0, "Hello everyone.");
        t.set(4, 19.0, "Let's start with the budget.");
        t
    }

    /// The stamp in the file is `[MM:SS]`, so a fractional time comes back rounded. That is fine —
    /// and has to be *deliberate*, because the alternative is a caller quietly building subtitles
    /// from a timing source that lost half a second per line.
    #[test]
    fn the_written_timestamp_is_second_precision_by_design() {
        let mut t = Translation::new("en");
        t.set(1, 19.5, "hi");
        let parsed = Translation::parse(&t.to_markdown()).unwrap();
        assert_eq!(parsed.lines[0].t0, 19.0);
    }

    #[test]
    fn a_translation_survives_a_round_trip() {
        let original = sample();
        let parsed = Translation::parse(&original.to_markdown()).unwrap();
        assert_eq!(parsed, original);
    }

    /// The whole reason lines carry `seq`: an edit to the transcript must not silently reattach
    /// every translation to the wrong sentence.
    #[test]
    fn a_line_is_found_by_its_segment_id_not_its_position() {
        let t = sample();
        assert_eq!(t.get(4), Some("Let's start with the budget."));
        assert_eq!(t.get(99), None);
    }

    #[test]
    fn retranslating_one_line_replaces_it_rather_than_appending() {
        let mut t = sample();
        t.set(3, 12.0, "Hi everyone.");
        assert_eq!(t.lines.len(), 2);
        assert_eq!(t.get(3), Some("Hi everyone."));
    }

    #[test]
    fn lines_stay_in_time_order_however_they_arrive() {
        let mut t = Translation::new("en");
        t.set(9, 90.0, "last");
        t.set(1, 1.0, "first");
        t.set(5, 45.0, "middle");
        let texts: Vec<_> = t.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["first", "middle", "last"]);
    }

    #[test]
    fn a_file_with_no_language_is_an_error_rather_than_an_empty_tag() {
        let err = Translation::parse("[00:01] hello <!-- seq:1 -->")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ngôn ngữ"), "{err}");
    }

    /// Someone will open the file and add a note. Losing it on the next save would be rude; so
    /// would attaching it to a random segment.
    #[test]
    fn a_hand_written_line_without_a_seq_is_skipped_not_guessed_at() {
        let text = "<!-- summo:translation lang:en -->\n\nJust a note I typed.\n[00:01] hi <!-- seq:1 -->\n";
        let t = Translation::parse(text).unwrap();
        assert_eq!(t.lines.len(), 1);
        assert_eq!(t.get(1), Some("hi"));
    }

    #[test]
    fn text_containing_a_comment_marker_does_not_swallow_the_seq() {
        let t = Translation::parse("<!-- summo:translation lang:en -->\n[00:01] a -- b <!-- seq:7 -->")
            .unwrap();
        assert_eq!(t.get(7), Some("a -- b"));
    }

    // A language tag arrives from an HTTP body; without this it names the file it is written to.
    #[test]
    fn a_language_tag_cannot_escape_the_translations_folder() {
        assert_eq!(sanitize_lang("../../settings"), "settings");
        assert_eq!(sanitize_lang("en/../x"), "enx");
        assert_eq!(sanitize_lang("en-US"), "en-us");
        assert_eq!(sanitize_lang("vi"), "vi");
    }

    #[test]
    fn a_tag_with_nothing_usable_in_it_is_refused() {
        let paths = Paths::at("/tmp/summo-x");
        assert!(path(&paths, &MeetingId::new(), "///").is_err());
    }

    #[test]
    fn saving_then_loading_returns_the_same_translation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();

        assert_eq!(load(&paths, &id, "en").unwrap(), None);
        save(&paths, &id, &sample()).unwrap();
        assert_eq!(load(&paths, &id, "en").unwrap(), Some(sample()));
    }

    #[test]
    fn a_meeting_lists_only_its_own_languages() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let mine = MeetingId::new();
        let theirs = MeetingId::new();

        save(&paths, &mine, &Translation::new("en")).unwrap();
        save(&paths, &mine, &Translation::new("ja")).unwrap();
        save(&paths, &theirs, &Translation::new("fr")).unwrap();

        assert_eq!(languages(&paths, &mine), ["en", "ja"]);
        assert_eq!(languages(&paths, &theirs), ["fr"]);
    }

    #[test]
    fn removing_a_translation_that_is_not_there_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();
        assert!(!remove(&paths, &id, "en").unwrap());

        save(&paths, &id, &sample()).unwrap();
        assert!(remove(&paths, &id, "en").unwrap());
        assert!(languages(&paths, &id).is_empty());
    }
}
