//! The board: every task in the vault, and moving them.
//!
//! Tasks live one-per-line across many Markdown files, so a board is assembled by reading the vault
//! rather than queried. That is affordable for the same reason the library listing is — the scan is
//! bounded by the number of meetings, not by their length — and it means a task edited in Obsidian
//! shows up here without anything having to notice.
//!
//! Writes go back to the exact line they came from ([`summo_vault::tasks::update`]), so a change to
//! one checkbox never reformats the notes around it.

use serde::{Deserialize, Serialize};
use summo_core::{Result, paths::Paths};
use summo_vault::tasks::{self, Status, Task};

// Re-exported so the daemon's handlers keep one import, and so it is obvious that writing a task
// is a vault operation rather than something the server invented.
pub use summo_vault::tasks_io::{create, update};

/// Tasks grouped the way the board draws them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Board {
    /// Work for people, in column order.
    pub todo: Vec<Task>,
    pub doing: Vec<Task>,
    pub done: Vec<Task>,
    pub blocked: Vec<Task>,
    /// Work the agent owns, kept separate — it is a different kind of thing with its own progress.
    pub agent: Vec<Task>,
    /// Everyone who owns at least one task, for the filter.
    pub owners: Vec<String>,
}

/// Read every task in the vault.
pub fn read(paths: &Paths) -> Result<Board> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::of_vault(&vault)?;

    let mut all = Vec::new();
    for entry in index.entries() {
        let path = vault.join(&entry.path);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };

        // A typed note is entirely the user's, so every checkbox in it is a task they meant. A
        // recorded meeting has a model-written summary, so only the actions section counts —
        // otherwise a checkbox the model produced inside "Quyết định" becomes a task nobody agreed
        // to. The document decides, not the folder: moving a file must not change what it means.
        //
        // The kind is the index's, which read it from the head of the file during the scan. This
        // used to open and parse every document in the vault a second time to ask the same
        // question, and to answer it differently from the three other places that ask it.
        all.extend(tasks::parse_document(
            &body,
            &entry.path.display().to_string(),
            entry.kind,
        ));
    }

    let mut owners: Vec<String> = all
        .iter()
        .filter_map(|t| t.owner.clone())
        .filter(|o| o != "agent")
        .collect();
    owners.sort();
    owners.dedup();

    let (agent, people): (Vec<Task>, Vec<Task>) = all.into_iter().partition(Task::is_agent);

    let mut board = Board {
        todo: Vec::new(),
        doing: Vec::new(),
        done: Vec::new(),
        blocked: Vec::new(),
        agent,
        owners,
    };
    for task in people {
        match task.status {
            Status::Todo => board.todo.push(task),
            Status::Doing => board.doing.push(task),
            Status::Done => board.done.push(task),
            // A failed task belongs next to the blocked ones: both are waiting on a human.
            Status::Blocked | Status::Failed => board.blocked.push(task),
        }
    }
    Ok(board)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let meetings = Paths::at(dir.path()).meetings();
        std::fs::create_dir_all(&meetings).unwrap();
        for (name, body) in files {
            std::fs::write(meetings.join(name), body).unwrap();
        }
        dir
    }

    fn meeting(id: &str, extra: &str) -> String {
        format!(
            "---\nid: {id}\ndate: 2026-08-10T09:00:00+07:00\nduration: 600\n\
             participants: []\ntags: []\n---\n# Họp {id}\n{extra}\n"
        )
    }

    const TASKS: &str = "## Việc cần làm\n\
        - [ ] @ngoc Chốt spec <!-- id:T1 status:doing -->\n\
        - [ ] @binh Gọi khách <!-- id:T2 -->\n\
        - [x] Gửi biên bản <!-- id:T3 -->\n\
        - [ ] @ngoc Chờ pháp lý <!-- id:T4 status:blocked -->\n\
        - [ ] @agent Tạo lịch <!-- id:T5 status:running -->\n\
        \x20 - [x] Quét ghi chú\n";

    #[test]
    fn an_empty_vault_yields_an_empty_board() {
        let dir = TempDir::new().unwrap();
        let board = read(&Paths::at(dir.path())).expect("read");
        assert!(board.todo.is_empty() && board.agent.is_empty() && board.owners.is_empty());
    }

    #[test]
    fn tasks_land_in_the_right_columns() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let board = read(&Paths::at(dir.path())).expect("read");

        assert_eq!(board.doing.len(), 1);
        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.done.len(), 1);
        assert_eq!(board.blocked.len(), 1);
        assert_eq!(board.agent.len(), 1, "agent work is its own board");
    }

    #[test]
    fn the_agent_keeps_its_own_steps() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let board = read(&Paths::at(dir.path())).expect("read");
        assert_eq!(board.agent[0].steps.len(), 1);
        assert_eq!(board.agent[0].progress(), Some(1.0));
    }

    #[test]
    fn owners_are_listed_without_the_agent() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let board = read(&Paths::at(dir.path())).expect("read");
        assert_eq!(board.owners, vec!["binh", "ngoc"]);
    }

    #[test]
    fn tasks_are_gathered_across_meetings() {
        let dir = vault(&[
            (
                "a.md",
                &meeting("01A", "## Việc cần làm\n- [ ] một <!-- id:A -->\n"),
            ),
            (
                "b.md",
                &meeting("01B", "## Việc cần làm\n- [ ] hai <!-- id:B -->\n"),
            ),
        ]);
        let board = read(&Paths::at(dir.path())).expect("read");
        assert_eq!(board.todo.len(), 2);
    }

    #[test]
    fn moving_a_task_writes_it_back() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let paths = Paths::at(dir.path());

        let moved = update(&paths, "T2", Some(Status::Done), None, None).expect("update");
        assert_eq!(moved.status, Status::Done);

        let board = read(&paths).expect("read");
        assert_eq!(board.done.len(), 2);
        assert!(board.todo.is_empty());
    }

    #[test]
    fn moving_a_task_leaves_the_rest_of_the_file_alone() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let paths = Paths::at(dir.path());
        update(&paths, "T1", Some(Status::Done), None, None).expect("update");

        let body = std::fs::read_to_string(paths.meetings().join("a.md")).unwrap();
        assert!(body.contains("- [ ] @binh Gọi khách"), "{body}");
        assert!(
            body.contains("  - [x] Quét ghi chú"),
            "the agent's steps kept their indent"
        );
        assert!(body.contains("# Họp 01A"));
    }

    #[test]
    fn reassigning_an_owner_works_and_clearing_it_works() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let paths = Paths::at(dir.path());

        let reassigned =
            update(&paths, "T2", None, Some(Some("ngoc".into())), None).expect("reassign");
        assert_eq!(reassigned.owner.as_deref(), Some("ngoc"));

        let cleared = update(&paths, "T2", None, Some(None), None).expect("clear");
        assert!(cleared.owner.is_none());
    }

    #[test]
    fn updating_a_task_that_is_not_there_is_an_error() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        assert!(
            update(
                &Paths::at(dir.path()),
                "NOPE",
                Some(Status::Done),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn creating_attaches_the_task_to_its_meeting() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        let paths = Paths::at(dir.path());

        let created = create(
            &paths,
            &summo_core::MeetingId::from("01A".to_string()),
            "  Việc mới  ",
            Some("binh"),
            Some("2026-08-20"),
        )
        .expect("create");

        assert_eq!(
            created.text, "Việc mới",
            "surrounding space is not part of the task"
        );
        assert_eq!(created.owner.as_deref(), Some("binh"));
        assert_eq!(created.due.as_deref(), Some("2026-08-20"));

        let board = read(&paths).expect("read");
        assert!(board.todo.iter().any(|t| t.id == created.id));
    }

    #[test]
    fn creating_in_a_meeting_without_a_task_section_still_works() {
        let dir = vault(&[("a.md", &meeting("01A", "## Tóm tắt\nChỉ có tóm tắt.\n"))]);
        let paths = Paths::at(dir.path());
        create(
            &paths,
            &summo_core::MeetingId::from("01A".to_string()),
            "Đầu tiên",
            None,
            None,
        )
        .expect("create");
        assert_eq!(read(&paths).expect("read").todo.len(), 1);
    }

    #[test]
    fn an_empty_task_is_refused() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        assert!(
            create(
                &Paths::at(dir.path()),
                &summo_core::MeetingId::from("01A".to_string()),
                "   ",
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn creating_in_a_meeting_that_does_not_exist_is_an_error() {
        let dir = vault(&[("a.md", &meeting("01A", TASKS))]);
        assert!(
            create(
                &Paths::at(dir.path()),
                &summo_core::MeetingId::from("01NOPE".to_string()),
                "x",
                None,
                None
            )
            .is_err()
        );
    }
}
