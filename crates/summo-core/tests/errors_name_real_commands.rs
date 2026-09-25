//! Every `summo <something>` an error message tells a user to run is a command that exists.
//!
//! Two shipped messages named commands nobody had:
//!
//! * `summo import` with no daemon said *"Mở app Summo, hoặc chạy `summo-engine`."* — the crate's
//!   name and the binary a checkout builds. The release ships one executable, called `summo`. So
//!   the first thing a new user was told to run did not exist on their machine.
//! * `summo dub` without a translation said *"run `summo translate` first"*. There has never been a
//!   `summo translate`, under any feature, in any release: translating a finished meeting is a
//!   daemon route reached from the meeting's export panel. The prerequisite for the one command
//!   that needs it named a command nobody could run.
//!
//! Both are the same mistake, and it is invisible in review — the sentence reads perfectly, and the
//! only way to notice is to type it. So this types it.
//!
//! The command list is parsed from `main.rs`'s `enum Command` rather than from clap at runtime,
//! because half the subcommands are behind Cargo features: asking the built binary what it can do
//! would make this test's answer depend on which features happened to be on, and a message about
//! `summo dub` is correct whether or not this build has dubbing in it.
//!
//! In `summo-core` for the same reason as `ci_builds_every_feature`: this crate declares no
//! features, so the test runs under the plain `cargo test --workspace` that gates a pull request.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Subcommand names, from the `enum Command` and `enum RegistryCmd` declarations in the CLI.
///
/// A variant line looks like `    Pull {` or `    List,`; clap lowercases the name. Anything with
/// an explicit `#[command(alias = "…")]` or `#[command(name = "…")]` is picked up too, because an
/// alias is a name a user can type and therefore a name an error may print.
fn commands() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    // `Meetings(library::MeetingCmd)` and `Registry(RegistryCmd)` each carry their own verbs, and
    // one of those enums lives in another file.
    for file in [
        "crates/summo-cli/src/main.rs",
        "crates/summo-cli/src/library.rs",
    ] {
        let source = std::fs::read_to_string(root().join(file))
            .unwrap_or_else(|e| panic!("{file} is where commands are declared: {e}"));

        let mut inside = false;
        for line in source.lines() {
            let start = line.trim_start();
            if start.starts_with("enum ") || start.starts_with("pub enum ") {
                inside = start.contains("Command") || start.contains("Cmd");
                continue;
            }
            if inside && line == "}" {
                inside = false;
                continue;
            }
            if !inside {
                continue;
            }

            // `alias = "vault"` — a second spelling of a command, equally real.
            if let Some(alias) = line
                .split("alias = \"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
            {
                names.insert(alias.to_string());
            }

            // A variant is four spaces, then an identifier, then `{`, `(`, `,` or nothing. Written
            // this way rather than by trimming the tail: `Rm { id: String },` is a whole variant on
            // one line, and so is `Registry(RegistryCmd),`.
            let Some(variant) = line.strip_prefix("    ") else {
                continue;
            };
            if !variant.starts_with(|c: char| c.is_ascii_uppercase()) {
                continue;
            }
            let name: String = variant
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if name.is_empty() {
                continue;
            }
            let after = variant[name.len()..].trim_start();
            if after.is_empty() || after.starts_with(['{', '(', ',']) {
                names.insert(name.to_lowercase());
            }
        }
    }

    for expected in [
        "pull", "serve", "dub", "registry", "meetings", "rm", "check",
    ] {
        assert!(
            names.contains(expected),
            "the command list did not parse — `{expected}` is missing from {names:?}"
        );
    }
    names
}

/// Every Rust source file in the workspace, excluding build output.
fn sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root().join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Not this file. Every example of a wrong command here is quoted on purpose, and a
                // test that fails on its own explanation of what it is for is a trap.
                if path
                    .file_name()
                    .is_some_and(|n| n == "errors_name_real_commands.rs")
                {
                    continue;
                }
                found.push(path);
            }
        }
    }
    found
}

/// Every `` `summo <word>` `` in a file, with the line it is on.
///
/// Backticked only. A sentence mentioning Summo by name is prose; a backticked `summo x` is an
/// instruction, and an instruction is the thing that has to be true.
fn instructions(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("`summo ") {
            rest = &rest[at + 1..];
            let Some(end) = rest.find('`') else { break };
            let inside = &rest[..end];
            // `summo pull <id>` and `summo registry check <dir>` — the verb is the second word,
            // and for `registry` the one after that is a verb too.
            let mut words = inside.split_whitespace().skip(1);
            if let Some(verb) = words.next() {
                found.push((number + 1, verb.to_string()));
                if verb == "registry"
                    && let Some(sub) = words.next()
                {
                    found.push((number + 1, sub.to_string()));
                }
            }
        }
    }
    found
}

#[test]
fn every_command_an_error_names_is_one_a_user_has() {
    let known = commands();
    let mut wrong = Vec::new();

    for path in sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (line, verb) in instructions(&text) {
            // A placeholder rather than a command: `summo <command>` in help text.
            if verb.starts_with('<') || verb.starts_with('[') {
                continue;
            }
            if !known.contains(&verb) {
                let shown = path
                    .strip_prefix(root())
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                wrong.push(format!("{shown}:{line}: `summo {verb}` is not a command"));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "an error message tells a user to run something that does not exist:\n  {}\n\nThe \
         commands are: {}",
        wrong.join("\n  "),
        known.iter().cloned().collect::<Vec<_>>().join(", ")
    );
}

/// The daemon is started by a command, not by a crate name.
///
/// Narrower than the test above and worth keeping separate: `summo-engine` is a real thing — a
/// crate, a binary in a checkout, a heading in the docs — so it will keep being written. What it is
/// not is something a person who downloaded a release can type.
#[test]
fn nothing_tells_a_user_to_run_the_engine_binary() {
    let mut wrong = Vec::new();
    for path in sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            if code.contains("chạy `summo-engine`") || code.contains("run `summo-engine`") {
                let shown = path
                    .strip_prefix(root())
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                wrong.push(format!("{shown}:{}", number + 1));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the release ships one executable and it is called `summo`; use `summo serve`:\n  {}",
        wrong.join("\n  ")
    );
}
