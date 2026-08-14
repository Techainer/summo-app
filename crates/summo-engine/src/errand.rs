//! Asking an agent to do something, without writing a checkbox first.
//!
//! Until now the only way to give an agent work was to type `- [ ] @agent …` into a meeting note.
//! That is a good *record* of the instruction and a poor way to give one: it means opening the
//! right note, remembering the syntax, and knowing which note counts. The assistant panel needs to
//! take a sentence from anywhere in the app and hand it to the same machinery.
//!
//! **The instruction still becomes a checkbox in the vault.** That is not a shortcut this module
//! takes; it is the point. The agent writes its steps into the file beside the task as it works
//! ([`summo_agent::steps`]), so a run has a readable trace in Markdown whether it was started from
//! a note or from a text box — and a user who wants to know what the agent did on Tuesday reads a
//! file rather than asking the application to remember.
//!
//! The scratch note is one per day, `Việc giao cho agent`, in `vault/notes/`. One per day rather
//! than one per instruction, so a week of small errands is seven files rather than eighty, and
//! rather than one file forever, because a file that only grows is one nobody opens twice.

use summo_core::{Error, MeetingId, Result, paths::Paths};
use summo_vault::tasks::Task;

/// Title of the day's scratch note. Also how it is found again, since the note has no other mark.
const SCRATCH: &str = "Việc giao cho agent";

/// Turn an instruction into a task the agent owns, and run it.
///
/// `agent` names one from `vault/agents/`; `None` uses the coordinator, which is what the roster is
/// for. Errors from the run itself are returned rather than swallowed: the panel shows them, and a
/// failed errand that looks like a finished one is worse than an error message.
pub async fn run(
    paths: &Paths,
    instruction: &str,
    agent: Option<&str>,
    meeting: Option<&str>,
) -> Result<summo_agent::run::Ran> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err(Error::msg("errand.empty", "muốn agent làm gì?"));
    }

    // Written down before the run, not after: an ask that failed is still what the person wanted,
    // and learning only from the ones that happened to work means the habit forms late or never.
    // A failure here costs a habit, never the errand.
    if let Err(e) = summo_agent::habits::record(&paths.agents(), &today(), instruction, meeting) {
        tracing::warn!(error = %e, "could not write down what was asked");
    }

    let note = scratch(paths, &today())?;
    // `@agent` is what marks a task as the agent's, and `Task::is_agent` is what `run_as` checks
    // before it will touch one. Creating it any other way would produce a task the runner refuses.
    let task = summo_vault::tasks_io::create(paths, &note, instruction, Some("agent"), None)?;
    summo_agent::run::run_as(paths, &task, agent).await
}

/// The day's scratch note, made if it is not there.
///
/// Found by title rather than by a marker in the frontmatter: the note is an ordinary note, and a
/// user who renames it has renamed it. A new one appears the next day and the old one keeps its
/// contents, which is the behaviour of a notebook rather than of a hidden cache.
fn scratch(paths: &Paths, day: &str) -> Result<MeetingId> {
    let existing = summo_vault::note::list(paths)?
        .into_iter()
        .find(|entry| entry.title == SCRATCH && entry.day == day);

    match existing {
        Some(entry) => Ok(entry.id),
        None => {
            let (id, _) = summo_vault::note::create(paths, SCRATCH, day, "")?;
            Ok(id)
        }
    }
}

/// Today, in local time.
///
/// Local rather than UTC for the same reason the recorder uses it: an errand at eleven at night
/// belongs to the day the person doing it thinks they are in.
fn today() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

/// What the panel shows while and after a run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Errand {
    pub task: String,
    pub outcome: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Step {
    pub text: String,
    pub done: bool,
}

impl From<summo_agent::run::Ran> for Errand {
    fn from(ran: summo_agent::run::Ran) -> Self {
        Self {
            task: ran.task,
            outcome: ran.outcome,
            steps: ran
                .steps
                .into_iter()
                .map(|step| Step {
                    text: step.text,
                    done: step.done,
                })
                .collect(),
        }
    }
}

/// The task an instruction became, for a caller that wants it without running anything.
#[must_use]
pub fn describe(task: &Task) -> String {
    task.text.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        paths.ensure().unwrap();
        (dir, paths)
    }

    #[test]
    fn the_days_scratch_note_is_made_once_and_found_again() {
        let (_dir, paths) = vault();
        let first = scratch(&paths, "2026-08-12").unwrap();
        let again = scratch(&paths, "2026-08-12").unwrap();
        assert_eq!(first, again, "a second errand joins the same note");

        // A new day is a new note, so a week of errands is seven files rather than one that only
        // grows.
        let tomorrow = scratch(&paths, "2026-08-13").unwrap();
        assert_ne!(first, tomorrow);
    }

    /// `run_as` refuses a task the agent does not own, so an instruction that arrived any other way
    /// would be created and then declined.
    #[test]
    fn an_instruction_becomes_a_task_the_agent_owns() {
        let (_dir, paths) = vault();
        let note = scratch(&paths, "2026-08-12").unwrap();
        let task =
            summo_vault::tasks_io::create(&paths, &note, "Tạo lịch", Some("agent"), None).unwrap();
        assert!(task.is_agent());
        assert_eq!(describe(&task), "Tạo lịch");
    }

    #[tokio::test]
    async fn an_empty_instruction_is_refused_before_a_note_is_touched() {
        let (_dir, paths) = vault();
        assert!(run(&paths, "   ", None, None).await.is_err());
        assert!(
            summo_vault::note::list(&paths).unwrap().is_empty(),
            "refusing must not leave a scratch note behind"
        );
    }
}
