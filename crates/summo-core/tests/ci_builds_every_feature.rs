//! Every feature a crate declares is built by CI, or is written down here with the reason it is
//! not.
//!
//! A Cargo feature decides what compiles. `cargo test --workspace` — the only `cargo test` a pull
//! request runs — uses default features, and every crate here defaults to none of them. So the
//! speech and model paths were compiled *out* of the one test command that gates merges, and it
//! reported a green tick over what was left:
//!
//! | crate | default | with its feature |
//! |---|---|---|
//! | `summo-engine` | 364 | 432 with `models` |
//! | `summo-asr` | 48 | 77 with `sherpa` |
//! | `summo-vad` | 15 | 18 with `silero` |
//! | `summo-cli` | 22 | 24 with `serve,engine,transcribe` |
//!
//! A hundred and two tests, including the ones that say a model chosen for one language does not
//! decide another and that deleting a model releases the roles pointing at it — both of which are
//! bugs a user reported. `cargo clippy --all-targets` did not cover them either, because it too
//! runs on default features: the `#[cfg(all(test, feature = "models"))]` module in `server.rs` was
//! never parsed by CI at all.
//!
//! This test lives in `summo-core` deliberately. That crate declares no features, so it compiles
//! and runs in exactly the default configuration it is here to guard — a guard behind a flag is
//! the thing being guarded against.
//!
//! It reads the workflows rather than a list kept beside them, for the same reason
//! `apps/web/e2e/suites.test.mjs` reads `ci.yml` rather than `package.json`: a script that exists
//! and is never invoked is precisely what went wrong.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository root, from this crate's manifest directory.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Features that are deliberately not built, each with the reason.
///
/// Written as `crate/feature` so a reader sees which crate is making the claim.
fn excused(name: &str) -> Option<&'static str> {
    Some(match name {
        // llama.cpp: a C++ toolchain and several minutes of CMake, for models that are not
        // redistributable anyway. The tag build is where it would be compiled, and it is not in
        // the shipped feature set. Stated in the `translation` job as well.
        "summo-mt/local" => "builds llama.cpp",
        "summo-engine/mt-gguf" => "builds llama.cpp",
        // Enabled by another feature and never named on a command line — `mt-any` is the marker
        // both translation runtimes turn on so shared code has one `cfg`, and `local-mt` is the
        // pair of them at once, which means llama.cpp again.
        "summo-engine/mt-any" => "a marker other features enable",
        "summo-engine/local-mt" => "both translation runtimes, so llama.cpp",
        "summo-cli/local-mt" => "both translation runtimes, so llama.cpp",
        // A bench harness for storage experiments that ships in nothing.
        "summo-bench/sqlite" => "an experiment, in no shipped binary",
        "summo-bench/turso" => "an experiment, in no shipped binary",
        _ => return None,
    })
}

/// Every `[features]` key a workspace crate declares, as `crate/feature`.
fn declared() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let crates = root().join("crates");
    for entry in std::fs::read_dir(&crates).expect("crates/ is readable") {
        let dir = entry.expect("a readable directory entry").path();
        let manifest = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a crate directory has a name")
            .to_owned();

        // The `[features]` table, up to the next table header. Parsed by hand rather than with a
        // TOML dependency: this is a test in the one crate that must stay dependency-light, and
        // the shape being read is four lines of `key = [...]`.
        let Some(start) = text.find("\n[features]\n") else {
            continue;
        };
        let table = &text[start + "\n[features]\n".len()..];
        let table = table.find("\n[").map_or(table, |end| &table[..end]);

        for line in table.lines() {
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty() || key == "default" || key.starts_with('#') {
                continue;
            }
            if !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            {
                continue;
            }
            out.insert(format!("{name}/{key}"));
        }
    }
    assert!(
        !out.is_empty(),
        "no features found — the parse is wrong, not the repository"
    );
    out
}

/// Whether CI ever *compiles the tests* for this feature.
///
/// The distinction is the whole point, and the first version of this test missed it. `models` was
/// already named in `cargo build --bin summo-engine --features bundled,models,mt-onnx,tts`, so a
/// check for "built somewhere" was satisfied by a command that builds one binary and no test at
/// all — it would have passed happily over the sixty-eight tests it exists to have noticed.
///
/// Only two kinds of command compile a `#[cfg(test)]` module: `cargo test`, and `cargo clippy`
/// with `--all-targets`. Everything else can have the feature on its command line and still never
/// look at a test.
///
/// Matched inside a `--features` list rather than anywhere in the file, so a feature named only in
/// a sentence of a comment does not count.
fn tested_by_ci(feature: &str) -> bool {
    let workflows = root().join(".github/workflows");
    let want = feature.split('/').next_back().expect("crate/feature");
    for entry in std::fs::read_dir(&workflows).expect("the workflows directory is readable") {
        let path = entry.expect("a readable directory entry").path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let compiles_tests = line.contains("cargo test")
                || (line.contains("cargo clippy") && line.contains("--all-targets"));
            if !compiles_tests {
                continue;
            }
            let Some(at) = line.find("--features") else {
                continue;
            };
            let list = line[at + "--features".len()..]
                .trim_start_matches([' ', '='])
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if list.split(',').any(|f| f == want) {
                return true;
            }
        }
    }
    false
}

#[test]
fn every_declared_feature_has_its_tests_compiled_or_is_excused() {
    let unbuilt: Vec<String> = declared()
        .into_iter()
        .filter(|f| excused(f).is_none() && !tested_by_ci(f))
        .collect();
    assert_eq!(
        unbuilt,
        Vec::<String>::new(),
        "these features have their tests compiled by no CI command, and no reason written down in {}",
        file!()
    );
}

#[test]
fn nothing_is_excused_that_no_longer_exists() {
    let declared = declared();
    let stale: Vec<&str> = [
        "summo-mt/local",
        "summo-engine/mt-gguf",
        "summo-engine/mt-any",
        "summo-engine/local-mt",
        "summo-cli/local-mt",
        "summo-bench/sqlite",
        "summo-bench/turso",
    ]
    .into_iter()
    .filter(|name| !declared.contains(*name))
    .collect();
    assert_eq!(
        stale,
        Vec::<&str>::new(),
        "excused features that no crate declares any more"
    );
}
