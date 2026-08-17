//! Reading and writing tasks across the vault.
//!
//! [`crate::tasks`] is pure string work over one document and is tested as such. This is the layer
//! that knows where documents are: it finds the file a task lives in, rewrites the one line, and
//! puts it back atomically.
//!
//! It sits in `summo-vault` rather than in the daemon because two callers need it — the HTTP board
//! and the agent's own tools — and the agent must not depend on the daemon that hosts it.

use summo_core::{Error, Result, paths::Paths};

use crate::tasks::{self, Status, Task};

/// Change one task, writing it back to the file it came from.
///
/// Re-reads the file and re-finds the task by id rather than trusting the line number the caller
/// sent: a board rendered a minute ago may be describing a file the user has since edited, and
/// writing to a stale line would overwrite an unrelated one.
pub fn update(
    paths: &Paths,
    id: &str,
    status: Option<Status>,
    owner: Option<Option<String>>,
    due: Option<Option<String>>,
) -> Result<Task> {
    let vault = paths.vault();
    let index = crate::index::MeetingIndex::of_vault(&vault)?;

    for entry in index.entries() {
        let relative = entry.path.display().to_string();
        let path = vault.join(&entry.path);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(mut task) = tasks::parse(&body, &relative)
            .into_iter()
            .find(|t| t.id == id)
        else {
            continue;
        };

        if let Some(status) = status {
            task.status = status;
        }
        if let Some(owner) = owner {
            task.owner = owner;
        }
        if let Some(due) = due {
            task.due = due;
        }

        let rewritten = tasks::update(&body, &task)?;
        crate::write::write_atomically(&path, rewritten.as_bytes())?;
        return Ok(task);
    }

    Err(Error::Other(format!("no task with id {id}")))
}

/// Add a task to a meeting's own list.
///
/// This is what "add to tasks" on a summary bullet does: the action item stays attached to the
/// meeting it came out of, so the transcript that justifies it is one click away.
pub fn create(
    paths: &Paths,
    meeting: &summo_core::MeetingId,
    text: &str,
    owner: Option<&str>,
    due: Option<&str>,
) -> Result<Task> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Other("a task needs a description".into()));
    }

    let vault = paths.vault();
    let index = crate::index::MeetingIndex::of_vault(&vault)?;
    let entry = index
        .entries()
        .iter()
        .find(|e| &e.id == meeting)
        .ok_or_else(|| Error::Other(format!("no meeting with id {meeting}")))?;

    let relative = entry.path.display().to_string();
    let path = vault.join(&entry.path);
    let body = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;

    let task = Task {
        id: summo_core::MeetingId::new().to_string(),
        text: text.to_string(),
        owner: owner.map(str::to_string),
        status: Status::Todo,
        due: due.map(str::to_string),
        steps: Vec::new(),
        file: relative.clone(),
        line: 0,
    };

    let rewritten = tasks::append(&body, &task);
    crate::write::write_atomically(&path, rewritten.as_bytes())?;

    // Return it as parsed back out, so the caller gets the real line number.
    tasks::parse(&rewritten, &relative)
        .into_iter()
        .find(|t| t.id == task.id)
        .ok_or_else(|| Error::Other("the task was written but could not be read back".into()))
}
