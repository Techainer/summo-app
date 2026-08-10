//! Interface translations the user added themselves.
//!
//! Summo ships Vietnamese and English. Everything else arrives as a JSON file dropped into
//! `~/.summo/locales/`, and that is the whole contribution process — no build step, no pull
//! request, no waiting for a release. A tool used across a dozen countries gets translated because
//! translating it is easier than filing an issue about it.
//!
//! ```json
//! // ~/.summo/locales/ja.json
//! {
//!   "label": "日本語",
//!   "strings": { "nav": { "record": "録音" } }
//! }
//! ```
//!
//! A file may also be the bare nested object, without the `label`/`strings` wrapper, because that is
//! what somebody copying `vi.json` and editing it will produce.
//!
//! **A partial file is normal and must work.** Ten translated strings should give ten translated
//! strings and Vietnamese for the rest — the interface layers this over the built-in catalogs, so
//! nothing here needs to be complete.
//!
//! **A broken file must not take the daemon down.** These are hand-edited by people who are not
//! programmers; a stray comma is the expected case. Bad files are skipped with a warning and the
//! rest still load.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use summo_core::{Result, paths::Paths};

/// One user-supplied language.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Locale {
    /// The language's name *in that language* — "日本語", not "Japanese". A user looking for their
    /// own language in a list cannot read the list to find it.
    pub label: String,
    /// The nested string tree, exactly as written.
    pub strings: serde_json::Value,
}

/// Read every locale file. Never fails on a bad file; the daemon must start regardless.
#[must_use]
pub fn load(paths: &Paths) -> BTreeMap<String, Locale> {
    read_dir(&paths.locales())
}

fn read_dir(dir: &Path) -> BTreeMap<String, Locale> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        // No directory is the normal case, not an error.
        return out;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_tag(code) {
            tracing::warn!(file = %path.display(), "skipping: the filename is not a language tag");
            continue;
        }

        match parse(&path) {
            Ok(locale) => {
                out.insert(code.to_string(), locale);
            }
            Err(e) => tracing::warn!(file = %path.display(), error = %e, "skipping a locale file"),
        }
    }
    out
}

fn parse(path: &Path) -> Result<Locale> {
    let raw = std::fs::read_to_string(path).map_err(|e| summo_core::Error::io(path, e))?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;

    let default_label = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string();

    // Both shapes: the documented wrapper, and the bare tree somebody gets by copying vi.json.
    let (label, strings) = match value.get("strings") {
        Some(strings) => (
            value
                .get("label")
                .and_then(|l| l.as_str())
                .unwrap_or(&default_label)
                .to_string(),
            strings.clone(),
        ),
        None => (default_label, value),
    };

    if !strings.is_object() {
        return Err(summo_core::Error::Other(
            "a locale file must be a JSON object of strings".into(),
        ));
    }

    Ok(Locale { label, strings })
}

/// Whether a filename could be a language tag.
///
/// Letters, digits and hyphens only. The tag becomes a key the interface looks up and, more to the
/// point, this is read from a directory anyone can write to — a file called `../../etc` should not
/// be treated as a language.
#[must_use]
pub fn is_tag(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 16
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn no_locales_directory_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(&Paths::at(tmp.path())).is_empty());
    }

    #[test]
    fn a_wrapped_file_is_read_with_its_label() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(
            &paths.locales(),
            "ja.json",
            r#"{ "label": "日本語", "strings": { "nav": { "record": "録音" } } }"#,
        );

        let loaded = load(&paths);
        let ja = loaded.get("ja").expect("ja");
        assert_eq!(ja.label, "日本語");
        assert_eq!(ja.strings["nav"]["record"], "録音");
    }

    /// What somebody actually does: copy `vi.json`, translate it, save it. There is no wrapper in
    /// that file, and refusing it would be refusing the obvious workflow.
    #[test]
    fn a_bare_string_tree_is_accepted_too() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(&paths.locales(), "fr.json", r#"{ "nav": { "record": "Enregistrer" } }"#);

        let loaded = load(&paths);
        assert_eq!(loaded["fr"].strings["nav"]["record"], "Enregistrer");
        assert_eq!(loaded["fr"].label, "fr", "no label, so the tag stands in");
    }

    /// These are hand-edited by people who are not programmers; a stray comma is the expected case,
    /// and it must cost that one language rather than the daemon.
    #[test]
    fn a_broken_file_is_skipped_and_the_others_still_load() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(&paths.locales(), "bad.json", "{ oops,,, }");
        write(&paths.locales(), "de.json", r#"{ "nav": { "record": "Aufnehmen" } }"#);

        let loaded = load(&paths);
        assert!(!loaded.contains_key("bad"));
        assert!(loaded.contains_key("de"));
    }

    #[test]
    fn a_file_that_is_not_an_object_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(&paths.locales(), "x.json", r#"["not", "a", "catalog"]"#);
        assert!(load(&paths).is_empty());
    }

    #[test]
    fn files_that_are_not_json_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(&paths.locales(), "notes.txt", "hello");
        assert!(load(&paths).is_empty());
    }

    // The directory is writable by anything running as the user; a filename is not a safe key
    // without checking it.
    #[test]
    fn a_filename_that_is_not_a_language_tag_is_refused() {
        assert!(is_tag("vi"));
        assert!(is_tag("en-GB"));
        assert!(!is_tag("../../etc/passwd"));
        assert!(!is_tag(""));
        assert!(!is_tag("a_very_long_language_tag_indeed"));
    }

    /// A partial file is the normal state of a contributed translation.
    #[test]
    fn a_file_with_one_string_in_it_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        write(&paths.locales(), "ko.json", r#"{ "nav": { "record": "녹음" } }"#);
        assert_eq!(load(&paths)["ko"].strings["nav"]["record"], "녹음");
    }
}
