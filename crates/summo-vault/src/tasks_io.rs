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
    text: Option<String>,
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
        // Rewording, which had no route at all: a task captured from a meeting came out of a
        // model's summary and is sometimes half a sentence, and the only fix was to open the
        // Markdown file. An empty rewrite is refused rather than applied — it would leave a
        // checkbox with no text, which parses as a task nobody can identify.
        if let Some(text) = text {
            let text = text.trim();
            if text.is_empty() {
                return Err(Error::Other("a task needs a description".into()));
            }
            task.text = text.to_string();
        }

        let rewritten = tasks::update(&body, &task)?;
        crate::write::write_atomically(&path, rewritten.as_bytes())?;
        return Ok(task);
    }

    Err(Error::Other(format!("no task with id {id}")))
}

/// The file a task with no meeting behind it lives in.
///
/// A fixed stem rather than a title lookup. The title is what a person reads and is therefore
/// theirs to rename; the file is what this has to find again tomorrow, and matching on a heading
/// somebody edited would silently start a second list.
const LOOSE_STEM: &str = "viec-can-lam";

/// The heading of that file, in the vault's own language.
///
/// Vietnamese like every other structure Summo writes — `## Việc cần làm`, `## Tóm tắt` — because
/// the vault is a folder of Markdown a person opens in Obsidian, and a document whose headings
/// follow the interface language would change shape when somebody switched it.
const LOOSE_TITLE: &str = "Việc cần làm";

/// Add a task that came from no meeting.
///
/// The board could only ever show work extracted from a recording: every route into it needed a
/// meeting id, so *"Việc cần làm thì không tạo mới riêng được"* was exactly right — there was no
/// document for such a task to live in. This gives it one, a standing note, created the first time
/// somebody writes a task down and reused forever after.
///
/// A note rather than a new kind of document, because the board already reads every checkbox in
/// every note ([`crate::tasks::parse_document`]), and because a list somebody can open, edit and
/// reorder in any Markdown editor is worth more than a private store this would have to invent.
pub fn create_loose(
    paths: &Paths,
    text: &str,
    owner: Option<&str>,
    due: Option<&str>,
) -> Result<Task> {
    create(paths, &loose_list(paths)?, text, owner, due)
}

/// The standing list, created if this is the first loose task.
fn loose_list(paths: &Paths) -> Result<summo_core::MeetingId> {
    let dir = crate::note::dir(paths);
    let path = dir.join(format!("{LOOSE_STEM}.md"));
    if path.exists() {
        let body = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        return Ok(crate::meeting::MeetingDoc::parse(&body)?.frontmatter.id);
    }

    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let id = summo_core::MeetingId::new();
    // Dated the day it was made, which is what every other document in the vault carries and what
    // the index sorts on. It is not a date the list is *about* — this one outlives any day.
    let day = summo_core::today();
    let doc = crate::meeting::MeetingDoc::new(
        crate::meeting::Frontmatter::new(id.clone(), &day),
        LOOSE_TITLE,
    );
    crate::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;
    Ok(id)
}

/// Remove a task from the file it lives in.
///
/// The line goes, and nothing around it moves — the same rule [`update`] follows and for the same
/// reason. An agent task takes its indented steps with it, since a plan without the task it plans
/// is orphaned text in somebody's notes.
pub fn remove(paths: &Paths, id: &str) -> Result<()> {
    let vault = paths.vault();
    let index = crate::index::MeetingIndex::of_vault(&vault)?;
    for entry in index.entries() {
        let path = vault.join(&entry.path);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let relative = entry.path.display().to_string();
        if !tasks::parse(&body, &relative).iter().any(|t| t.id == id) {
            continue;
        }
        let rewritten = tasks::remove(&body, id);
        crate::write::write_atomically(&path, rewritten.as_bytes())?;
        return Ok(());
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

#[cfg(test)]
mod loose_tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> TempDir {
        TempDir::new().unwrap()
    }

    /// The gap the whole board had: no way to write down a task that no meeting produced.
    #[test]
    fn a_task_with_no_meeting_gets_a_list_of_its_own() {
        let dir = vault();
        let paths = Paths::at(dir.path());

        let made = create_loose(&paths, "Gọi ngân hàng", Some("viet"), None).expect("create");
        assert_eq!(made.text, "Gọi ngân hàng");
        assert_eq!(made.status, Status::Todo);

        let board = crate::tasks::parse(
            &std::fs::read_to_string(crate::note::dir(&paths).join("viec-can-lam.md")).unwrap(),
            "viec-can-lam.md",
        );
        assert_eq!(board.len(), 1);
    }

    /// And the second one joins the first rather than starting a second list.
    #[test]
    fn the_standing_list_is_made_once() {
        let dir = vault();
        let paths = Paths::at(dir.path());
        create_loose(&paths, "một", None, None).expect("first");
        create_loose(&paths, "hai", None, None).expect("second");

        let notes: Vec<_> = std::fs::read_dir(crate::note::dir(&paths))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
    }

    /// Rewording, which had no route at all.
    #[test]
    fn a_task_can_be_rewritten() {
        let dir = vault();
        let paths = Paths::at(dir.path());
        let made = create_loose(&paths, "Gọi", None, None).expect("create");

        let edited = update(
            &paths,
            &made.id,
            None,
            None,
            None,
            Some("Gọi ngân hàng".into()),
        )
        .expect("reword");
        assert_eq!(edited.text, "Gọi ngân hàng");
    }

    /// An empty rewrite would leave a checkbox nobody can identify.
    #[test]
    fn a_task_cannot_be_rewritten_to_nothing() {
        let dir = vault();
        let paths = Paths::at(dir.path());
        let made = create_loose(&paths, "Gọi", None, None).expect("create");
        assert!(update(&paths, &made.id, None, None, None, Some("   ".into())).is_err());
    }

    /// Deleting, which the board could only fake by dragging a non-task to "done".
    #[test]
    fn a_task_can_be_taken_off_the_board() {
        let dir = vault();
        let paths = Paths::at(dir.path());
        let keep = create_loose(&paths, "giữ", None, None).expect("create");
        let drop = create_loose(&paths, "bỏ", None, None).expect("create");

        remove(&paths, &drop.id).expect("remove");

        let body =
            std::fs::read_to_string(crate::note::dir(&paths).join("viec-can-lam.md")).unwrap();
        let left = crate::tasks::parse(&body, "viec-can-lam.md");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, keep.id);
    }

    #[test]
    fn deleting_a_task_that_is_not_there_is_an_error() {
        let dir = vault();
        let paths = Paths::at(dir.path());
        create_loose(&paths, "giữ", None, None).expect("create");
        assert!(remove(&paths, "NOPE").is_err());
    }
}
