//! A setting somebody can change, and nothing reads.
//!
//! `settings.json` is a file users edit by hand, and every field in it is a promise: change this
//! and the app behaves differently. Two fields once broke that promise — `compact_while_recording`
//! and `show_performance` — and they broke it *silently*. Nothing in the daemon, the app or the
//! desktop shell ever read either one. They validated, they defaulted, they round-tripped through
//! every test the settings module had, and they named features that did not exist.
//!
//! Both were deleted, and deleting them was right. What was missing is the thing that would have
//! caught them on the day they landed, and would catch the next one: a check that every field a
//! user can set is read by somebody.
//!
//! ## What counts as read
//!
//! A mention of the field's name outside the module that declares it. That is deliberately weak —
//! it cannot tell a real use from a log line — and it is still enough, because the failure it is
//! aimed at is *total*: a dead field has **zero** mentions anywhere. A field that is mentioned and
//! misused is a different bug with a different test, usually in the crate that misuses it.
//!
//! The search covers the interface as well as the crates. Several settings are read only by the
//! app — `interface.theme` is applied by the browser — and a check that looked at Rust alone would
//! call them dead and be wrong.

use std::path::{Path, PathBuf};

use summo_core::Settings;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Every source file that could read a setting: the Rust crates, and the interface.
fn sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root().join("crates"), root().join("apps")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                // `target`, `node_modules` and `dist` hold generated copies of the very files being
                // searched, so walking them turns every answer into "yes" — including for a field
                // that is only mentioned in a stale build from before it was deleted.
                if matches!(&*name, "target" | "node_modules" | "dist" | ".git") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "rs" | "ts" | "tsx" | "mjs") {
                found.push(path);
            }
        }
    }
    found
}

/// The file that declares the settings. Its own mentions do not count as anybody reading them.
fn declares(path: &Path) -> bool {
    path.ends_with("summo-core/src/settings.rs")
}

/// Every leaf field name in the settings, without its group.
///
/// `keys()` returns `interface.theme`; what a reader writes is `theme`, or `.theme`, or
/// `settings.interface.theme`. The leaf is the part every spelling has in common.
fn leaves() -> Vec<(String, String)> {
    Settings::default()
        .keys()
        .into_iter()
        .filter_map(|key| {
            let leaf = key.rsplit('.').next()?.to_string();
            Some((key, leaf))
        })
        // `schema` is the file's own version number and `unknown` is the bag that preserves a
        // newer build's fields. Neither is a setting anybody changes to change behaviour, and both
        // are read by the loader in the declaring file, which is excluded.
        .filter(|(_, leaf)| !matches!(leaf.as_str(), "schema" | "unknown"))
        .collect()
}

#[test]
fn every_setting_a_user_can_change_is_read_by_something() {
    let files: Vec<(PathBuf, String)> = sources()
        .into_iter()
        .filter(|path| !declares(path))
        .filter_map(|path| std::fs::read_to_string(&path).ok().map(|text| (path, text)))
        .collect();
    assert!(
        files.len() > 50,
        "the source walk found {} files — has the layout moved?",
        files.len()
    );

    let mut dead = Vec::new();
    for (key, leaf) in leaves() {
        let read = files.iter().any(|(_, text)| text.contains(&leaf));
        if !read {
            dead.push(key);
        }
    }

    assert!(
        dead.is_empty(),
        "these settings can be changed and nothing anywhere reads them: {dead:?}\n\
         A field in settings.json that changes nothing is worse than no field — the user edits it, \
         nothing happens, and there is no way to find out why. Either wire it up or delete it."
    );
}

/// The settings screen has a section per group, and a group with no section is unreachable.
///
/// The other half of the same promise. A field can be read by the daemon and still be impossible
/// to change without a text editor, which is where `storage.audio_retention_days` sat for several
/// releases — enforced on every prune, and settable only by editing the file.
#[test]
fn every_group_of_settings_has_somewhere_to_change_it() {
    let screen = root().join("apps/web/src/lib/settings.ts");
    let listed = std::fs::read_to_string(&screen)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", screen.display()));

    // Which settings group each section of the screen is responsible for. Stated here rather than
    // derived, because the names genuinely differ — `general` covers `interface`, and `ai` and
    // `translation` share `llm` and `models` between them.
    let covered: &[(&str, &str)] = &[
        ("recording", "recording"),
        ("models", "recording"),
        ("llm", "ai"),
        ("storage", "storage"),
        ("interface", "general"),
        ("agents", "ai"),
        ("sync", "sync"),
    ];

    let groups: Vec<String> = Settings::default()
        .keys()
        .into_iter()
        .filter_map(|key| key.split('.').next().map(str::to_string))
        .filter(|group| !matches!(group.as_str(), "schema" | "unknown"))
        .collect();

    let mut unreachable = Vec::new();
    for group in groups {
        let Some((_, section)) = covered.iter().find(|(name, _)| *name == group) else {
            unreachable.push(format!("{group} (no section claims it)"));
            continue;
        };
        if !listed.contains(&format!("\"{section}\"")) {
            unreachable.push(format!(
                "{group} → section `{section}`, which the screen does not have"
            ));
        }
    }

    assert!(
        unreachable.is_empty(),
        "these settings groups cannot be changed from the app: {unreachable:?}\n\
         Add a section to apps/web/src/lib/settings.ts and a panel beside it, or add the group to \
         the table in this test with the section that covers it."
    );
}
