//! What an agent remembers, and the tasks it keeps for itself.
//!
//! Both are Markdown files inside the agent's own directory, for the reasons in [`crate::roster`].
//! Two consequences worth stating, because they are the whole point:
//!
//! * A user can read what an agent believes, and **delete a line they disagree with**. Memory that
//!   can only be added to is memory that compounds its own mistakes.
//! * An agent can maintain its own list. `TASKS.md` is parsed by the same code that parses a
//!   meeting's action items, so an agent's checklist and a person's are the same object, and the
//!   board can show both without a second implementation.
//!
//! ## Why memory is a bulleted list and not a store
//!
//! The temptation is embeddings: remember everything, retrieve what is relevant. That is the wrong
//! trade here. An agent's useful memory is small — who people are, how this team names things, what
//! the user asked it to stop doing — and it is small precisely because it is *curated*. A store
//! that never forgets fills with the contents of meetings, which are already in the vault, and then
//! needs retrieval to find anything, and then the retrieval is what is wrong.
//!
//! A capped list the user can edit is a stronger design than a database they cannot.

use std::path::Path;

use summo_core::{Error, Result};

/// How many lines of memory to keep.
///
/// Old lines fall off the top. The cap exists so the system prompt cannot grow without bound — an
/// agent that has run for a year should not cost more per call than one that has run for a week —
/// and it is generous enough that the limit is reached by an agent that remembers indiscriminately,
/// which is a habit worth capping.
pub const MAX_LINES: usize = 60;

/// One remembered fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// ISO date it was learned, for the user's sake — an agent should not be trusted about *when*.
    pub learned: String,
    pub text: String,
}

/// Read an agent's memory as the lines that go into its prompt.
///
/// Anything that is not a bullet is ignored, so the file can carry a heading and the user's own
/// notes without those becoming instructions.
pub fn load(path: &Path) -> Vec<Fact> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Fact> {
    let rest = line.trim().strip_prefix("- ")?;
    // `- 2026-08-11 — fact`, the shape `remember` writes. A bullet without a date is still a fact;
    // the user wrote it by hand and should not have to match a format.
    match rest.split_once(" — ") {
        Some((date, text)) if is_date(date) => Some(Fact {
            learned: date.to_string(),
            text: text.trim().to_string(),
        }),
        _ => Some(Fact {
            learned: String::new(),
            text: rest.trim().to_string(),
        }),
    }
    .filter(|f| !f.text.is_empty())
}

fn is_date(text: &str) -> bool {
    text.len() == 10 && text.as_bytes()[4] == b'-' && text.as_bytes()[7] == b'-'
}

/// The memory as it appears in a system prompt.
#[must_use]
pub fn render(facts: &[Fact]) -> String {
    facts
        .iter()
        .map(|f| format!("- {}", f.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Append a fact, if it is not already there.
///
/// `today` is passed in rather than read from the clock, because a function that stamps the current
/// date cannot be tested for what it does at a month boundary — and because the caller already
/// knows what day the vault thinks it is.
///
/// Returns whether anything was written. A duplicate is not an error: an agent re-learning something
/// it already knows is normal, and failing the tool call would make it retry.
pub fn remember(path: &Path, today: &str, text: &str) -> Result<bool> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(false);
    }

    let mut facts = load(path);
    if facts.iter().any(|f| f.text.eq_ignore_ascii_case(text)) {
        return Ok(false);
    }
    facts.push(Fact {
        learned: today.to_string(),
        text: text.to_string(),
    });

    // Oldest first out. Keeping the newest is the right end to keep: what an agent learned most
    // recently is what the user most recently corrected it about.
    let overflow = facts.len().saturating_sub(MAX_LINES);
    let kept = &facts[overflow..];

    let mut out = String::from("# Memory\n\n");
    for fact in kept {
        if fact.learned.is_empty() {
            out.push_str(&format!("- {}\n", fact.text));
        } else {
            out.push_str(&format!("- {} — {}\n", fact.learned, fact.text));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, out).map_err(|e| Error::io(path, e))?;
    Ok(true)
}

/// Forget one fact, by exact text. What makes the memory the user's rather than the agent's.
pub fn forget(path: &Path, text: &str) -> Result<bool> {
    let facts = load(path);
    let kept: Vec<Fact> = facts
        .iter()
        .filter(|f| !f.text.eq_ignore_ascii_case(text.trim()))
        .cloned()
        .collect();
    if kept.len() == facts.len() {
        return Ok(false);
    }

    let mut out = String::from("# Memory\n\n");
    for fact in &kept {
        if fact.learned.is_empty() {
            out.push_str(&format!("- {}\n", fact.text));
        } else {
            out.push_str(&format!("- {} — {}\n", fact.learned, fact.text));
        }
    }
    std::fs::write(path, out).map_err(|e| Error::io(path, e))?;
    Ok(true)
}

/// An agent's own checklist.
///
/// Parsed by the vault's task parser, with `Everywhere` scope: this file is entirely the agent's
/// own, so every checkbox in it is a task it meant. That is the same rule a hand-written note gets,
/// and for the same reason.
#[must_use]
pub fn tasks(path: &Path, file_label: &str) -> Vec<summo_vault::tasks::Task> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    summo_vault::tasks::parse_scoped(&text, file_label, summo_vault::tasks::Scope::Everywhere)
}

/// Add a task to an agent's own list.
pub fn add_task(path: &Path, task: &summo_vault::tasks::Task) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_else(|_| "# Tasks\n\n".to_string());
    let updated = summo_vault::tasks::append(&existing, task);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, updated).map_err(|e| Error::io(path, e))
}

/// Change the status of one of an agent's own tasks.
pub fn set_status(
    path: &Path,
    id: &str,
    status: summo_vault::tasks::Status,
    file_label: &str,
) -> Result<bool> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(false);
    };
    let mut found = summo_vault::tasks::parse_scoped(
        &text,
        file_label,
        summo_vault::tasks::Scope::Everywhere,
    );
    let Some(task) = found.iter_mut().find(|t| t.id == id) else {
        return Ok(false);
    };
    task.status = status;
    let updated = summo_vault::tasks::update(&text, task)?;
    std::fs::write(path, updated).map_err(|e| Error::io(path, e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("MEMORY.md");
        (tmp, path)
    }

    #[test]
    fn a_fact_survives_a_round_trip() {
        let (_tmp, path) = file();
        assert!(remember(&path, "2026-08-11", "Ngọc leads product").unwrap());

        let facts = load(&path);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "Ngọc leads product");
        assert_eq!(facts[0].learned, "2026-08-11");
    }

    /// An agent re-learning what it knows is normal. Failing the call would make it retry.
    #[test]
    fn remembering_the_same_thing_twice_writes_once() {
        let (_tmp, path) = file();
        assert!(remember(&path, "2026-08-11", "Ngọc leads product").unwrap());
        assert!(!remember(&path, "2026-08-12", "ngọc LEADS product").unwrap());
        assert_eq!(load(&path).len(), 1);
    }

    #[test]
    fn an_empty_fact_is_not_remembered() {
        let (_tmp, path) = file();
        assert!(!remember(&path, "2026-08-11", "   ").unwrap());
        assert!(load(&path).is_empty());
    }

    /// Memory the user cannot edit is memory that compounds its own mistakes.
    #[test]
    fn a_user_can_delete_a_line_they_disagree_with() {
        let (_tmp, path) = file();
        remember(&path, "2026-08-11", "Ngọc leads product").unwrap();
        remember(&path, "2026-08-11", "Minh is on holiday").unwrap();

        assert!(forget(&path, "Minh is on holiday").unwrap());
        let facts = load(&path);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "Ngọc leads product");
    }

    #[test]
    fn forgetting_something_that_is_not_there_is_not_an_error() {
        let (_tmp, path) = file();
        remember(&path, "2026-08-11", "a").unwrap();
        assert!(!forget(&path, "b").unwrap());
        assert_eq!(load(&path).len(), 1);
    }

    /// A prompt that grows without bound makes a long-running agent cost more per call than a new
    /// one, for no benefit the user asked for.
    #[test]
    fn memory_is_capped_and_keeps_the_newest() {
        let (_tmp, path) = file();
        for i in 0..MAX_LINES + 10 {
            remember(&path, "2026-08-11", &format!("fact {i}")).unwrap();
        }
        let facts = load(&path);
        assert_eq!(facts.len(), MAX_LINES);
        assert_eq!(facts.last().unwrap().text, format!("fact {}", MAX_LINES + 9));
        assert!(
            !facts.iter().any(|f| f.text == "fact 0"),
            "the oldest should have fallen off"
        );
    }

    /// The file has to stay a document, not a format. A heading and a paragraph of the user's own
    /// must not become instructions.
    #[test]
    fn only_bullets_are_facts() {
        let (_tmp, path) = file();
        std::fs::write(
            &path,
            "# Memory\n\nSome notes I keep here myself.\n\n- 2026-08-11 — a fact\n",
        )
        .unwrap();
        let facts = load(&path);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "a fact");
    }

    /// Somebody typing a line by hand should not have to match a date format.
    #[test]
    fn a_hand_written_bullet_with_no_date_is_still_a_fact() {
        let (_tmp, path) = file();
        std::fs::write(&path, "- the user prefers short summaries\n").unwrap();
        let facts = load(&path);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "the user prefers short summaries");
        assert!(facts[0].learned.is_empty());
    }

    #[test]
    fn a_missing_file_is_an_empty_memory() {
        assert!(load(std::path::Path::new("/nonexistent/MEMORY.md")).is_empty());
    }

    #[test]
    fn rendering_drops_the_dates_the_model_does_not_need() {
        let facts = vec![Fact {
            learned: "2026-08-11".into(),
            text: "Ngọc leads product".into(),
        }];
        assert_eq!(render(&facts), "- Ngọc leads product");
    }

    // ---- an agent's own tasks ---------------------------------------------------------------

    #[test]
    fn an_agent_keeps_its_own_checklist() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("TASKS.md");
        std::fs::write(
            &path,
            "# Tasks\n\n- [ ] Re-read last week's standups <!-- id:A1 status:todo -->\n- [x] Learn the team's names <!-- id:A2 status:done -->\n",
        )
        .unwrap();

        let tasks = tasks(&path, "agents/librarian/TASKS.md");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "A1");
        assert!(tasks[1].status.is_finished());
    }

    #[test]
    fn an_agent_can_add_to_its_own_list_and_tick_it_off() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("TASKS.md");
        let task = summo_vault::tasks::Task {
            id: "A9".into(),
            text: "Check the glossary".into(),
            owner: Some("agent".into()),
            status: summo_vault::tasks::Status::Todo,
            due: None,
            steps: Vec::new(),
            file: "agents/scribe/TASKS.md".into(),
            line: 0,
        };
        add_task(&path, &task).unwrap();
        assert_eq!(tasks(&path, "x").len(), 1);

        assert!(
            set_status(&path, "A9", summo_vault::tasks::Status::Done, "x").unwrap(),
            "the task it just added must be findable"
        );
        assert!(tasks(&path, "x")[0].status.is_finished());
    }

    #[test]
    fn ticking_off_a_task_that_is_not_there_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("TASKS.md");
        std::fs::write(&path, "# Tasks\n\n").unwrap();
        assert!(!set_status(&path, "nope", summo_vault::tasks::Status::Done, "x").unwrap());
    }
}
