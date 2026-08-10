//! Accepting what the agent proposed.
//!
//! [`summo_vault::annotate`] models the conversation; this is the part that acts on it. Separating
//! them is not ceremony: it means the vault can hold a proposal without anything being able to
//! apply it by accident, and every path that changes a user's notes on the agent's suggestion goes
//! through one function that can be read in full.
//!
//! Three properties, all of which exist because the alternative is worse:
//!
//! * **Accepting is the only way an agent's suggestion reaches a note.** Nothing applies on a
//!   timer, on a retry, or because a second agent read the first one's proposal.
//! * **Accepting is recorded before it is performed.** If applying fails halfway, the proposal
//!   reads `accepted` and the error is reported — better than a proposal that still looks open
//!   while half of it has already happened.
//! * **A proposal is decided once.** Double-clicking accept must not create two tasks.

use serde::Serialize;
use summo_core::{Error, Result, paths::Paths};
use summo_vault::annotate::{self, Action, Anchor, Annotation, Kind, Resolution};
use summo_vault::tasks::Status;

/// What accepting a proposal did.
#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    pub annotation: String,
    /// Human-readable, the same wording the button offered.
    pub did: String,
    /// Task the action created or changed, when it was about a task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Accept a proposal and carry it out.
pub fn accept(paths: &Paths, note: &str, annotation_id: &str) -> Result<Applied> {
    let mut thread = annotate::load(paths, note)?;
    // `resolve` refuses a second decision, which is what stops a double click doing it twice.
    let decided = thread.resolve(annotation_id, Resolution::Accepted)?;
    annotate::save(paths, note, &thread)?;

    let action = decided
        .action
        .as_ref()
        .ok_or_else(|| Error::Other(format!("proposal {annotation_id} carries no action")))?;

    let did = action.describe();
    let task = perform(paths, note, action)?;
    Ok(Applied {
        annotation: annotation_id.to_string(),
        did,
        task,
    })
}

/// Dismiss a proposal without doing it.
pub fn dismiss(paths: &Paths, note: &str, annotation_id: &str) -> Result<Annotation> {
    let mut thread = annotate::load(paths, note)?;
    let decided = thread.resolve(annotation_id, Resolution::Dismissed)?;
    annotate::save(paths, note, &thread)?;
    Ok(decided)
}

fn perform(paths: &Paths, note: &str, action: &Action) -> Result<Option<String>> {
    match action {
        Action::CreateTask { text, owner, due } => {
            let task = summo_vault::tasks_io::create(
                paths,
                &summo_core::MeetingId::from(note.to_string()),
                text,
                owner.as_deref(),
                due.as_deref(),
            )?;
            Ok(Some(task.id))
        }
        Action::UpdateTask { id, status, owner } => {
            let status = status.as_deref().and_then(parse_status);
            let owner = owner.clone().map(Some);
            let task = summo_vault::tasks_io::update(paths, id, status, owner, None)?;
            Ok(Some(task.id))
        }
        Action::SetSection { heading, body } => {
            set_section(paths, note, heading, body)?;
            Ok(None)
        }
    }
}

fn parse_status(value: &str) -> Option<Status> {
    match value {
        "todo" => Some(Status::Todo),
        "doing" => Some(Status::Doing),
        "done" => Some(Status::Done),
        "blocked" => Some(Status::Blocked),
        "failed" => Some(Status::Failed),
        _ => None,
    }
}

fn set_section(paths: &Paths, note: &str, heading: &str, body: &str) -> Result<()> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;
    let entry = index
        .entries()
        .iter()
        .find(|e| e.id.as_str() == note)
        .ok_or_else(|| Error::Other(format!("no meeting with id {note}")))?;

    let path = vault.join(&entry.path);
    let markdown = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let mut doc = summo_vault::meeting::MeetingDoc::parse(&markdown)?;
    doc.set_section(heading, body);
    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())
}

/// Everything waiting on the user, across the whole vault.
///
/// This is what a "3 việc cần bạn duyệt" badge counts, and what the agent's proactive nudge reads.
/// Assembled by scanning rather than kept in an index, for the same reason the board is: it stays
/// correct when a file is edited outside the app.
pub fn pending(paths: &Paths) -> Result<Vec<(String, Annotation)>> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;

    let mut out = Vec::new();
    for entry in index.entries() {
        let note = entry.id.to_string();
        let Ok(thread) = annotate::load(paths, &note) else {
            // One unreadable thread must not hide every other pending decision.
            continue;
        };
        for annotation in thread.pending() {
            out.push((note.clone(), annotation.clone()));
        }
    }
    out.sort_by(|a, b| a.1.at.cmp(&b.1.at));
    Ok(out)
}

/// One thing somebody wants to add to a note's conversation.
///
/// A struct rather than eight positional arguments: at a call site,
/// `say(paths, id, Comment, "Ngọc", body, Note, None, now)` is unreadable, and the two `Option`s
/// next to each other are exactly where an argument gets passed in the wrong slot.
#[derive(Debug, Clone)]
pub struct Saying<'a> {
    pub note: &'a str,
    pub kind: Kind,
    /// Display name, or `agent`.
    pub author: &'a str,
    pub body: &'a str,
    pub anchor: Anchor,
    /// Required on a proposal, meaningless otherwise.
    pub action: Option<Action>,
    /// ISO-8601, in the offset it was written in.
    pub at: &'a str,
}

/// Add something to a note's conversation.
pub fn say(paths: &Paths, saying: Saying<'_>) -> Result<Annotation> {
    let Saying {
        note,
        kind,
        author,
        body,
        anchor,
        action,
        at,
    } = saying;

    if body.trim().is_empty() && action.is_none() {
        return Err(Error::Other("an annotation needs something to say".into()));
    }
    if kind == Kind::Proposal && action.is_none() {
        return Err(Error::Other(
            "a proposal must carry the action it is proposing, or there is nothing to accept".into(),
        ));
    }

    let mut thread = annotate::load(paths, note)?;
    let annotation = Annotation {
        id: summo_core::MeetingId::new().to_string(),
        kind,
        author: author.to_string(),
        at: at.to_string(),
        body: body.trim().to_string(),
        anchor,
        action,
        resolution: Resolution::Open,
        reactions: Vec::new(),
    };
    thread.annotations.push(annotation.clone());
    annotate::save(paths, note, &thread)?;
    Ok(annotation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault(tasks_block: &str) -> (TempDir, Paths) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(
            paths.meetings().join("01A.md"),
            format!(
                "---\nid: 01A\ndate: 2026-08-10T09:00:00+07:00\nduration: 600\n\
                 participants: []\ntags: []\n---\n# Họp\n\n## Tóm tắt\nCũ.\n\n{tasks_block}"
            ),
        )
        .unwrap();
        (dir, paths)
    }

    fn propose(paths: &Paths, action: Action) -> Annotation {
        say(
            paths,
            Saying {
                note: "01A",
                kind: Kind::Proposal,
                author: "agent",
                body: "Nghe thấy ai đó nhận việc này",
                anchor: Anchor::Note,
                action: Some(action),
                at: "2026-08-10T10:00:00+07:00",
            },
        )
        .expect("propose")
    }

    #[test]
    fn accepting_a_proposal_creates_the_task_it_described() {
        let (_d, paths) = vault("");
        let proposal = propose(
            &paths,
            Action::CreateTask {
                text: "Gửi báo giá".into(),
                owner: Some("binh".into()),
                due: None,
            },
        );

        let applied = accept(&paths, "01A", &proposal.id).expect("accept");
        assert!(applied.did.contains("Gửi báo giá"));
        assert!(applied.task.is_some());

        let board = crate::board::read(&paths).expect("board");
        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.todo[0].owner.as_deref(), Some("binh"));
    }

    /// The whole point: nothing happens until a person says so.
    #[test]
    fn a_proposal_changes_nothing_until_it_is_accepted() {
        let (_d, paths) = vault("");
        propose(
            &paths,
            Action::CreateTask {
                text: "Gửi báo giá".into(),
                owner: None,
                due: None,
            },
        );
        assert!(crate::board::read(&paths).expect("board").todo.is_empty());
    }

    #[test]
    fn dismissing_changes_nothing_and_closes_it() {
        let (_d, paths) = vault("");
        let proposal = propose(
            &paths,
            Action::CreateTask {
                text: "Không cần".into(),
                owner: None,
                due: None,
            },
        );

        dismiss(&paths, "01A", &proposal.id).expect("dismiss");
        assert!(crate::board::read(&paths).expect("board").todo.is_empty());
        assert!(pending(&paths).expect("pending").is_empty());
    }

    /// Double-clicking accept must not create the task twice.
    #[test]
    fn a_proposal_can_only_be_accepted_once() {
        let (_d, paths) = vault("");
        let proposal = propose(
            &paths,
            Action::CreateTask {
                text: "Một lần thôi".into(),
                owner: None,
                due: None,
            },
        );

        accept(&paths, "01A", &proposal.id).expect("first");
        assert!(accept(&paths, "01A", &proposal.id).is_err(), "accepted twice");
        assert_eq!(crate::board::read(&paths).expect("board").todo.len(), 1);
    }

    #[test]
    fn accepting_can_move_an_existing_task() {
        let (_d, paths) = vault("## Việc cần làm\n- [ ] @ngoc Cũ <!-- id:T1 -->\n");
        let proposal = propose(
            &paths,
            Action::UpdateTask {
                id: "T1".into(),
                status: Some("done".into()),
                owner: None,
            },
        );

        accept(&paths, "01A", &proposal.id).expect("accept");
        assert_eq!(crate::board::read(&paths).expect("board").done.len(), 1);
    }

    #[test]
    fn accepting_can_rewrite_a_section() {
        let (_d, paths) = vault("");
        let proposal = propose(
            &paths,
            Action::SetSection {
                heading: "Tóm tắt".into(),
                body: "Mới và đúng hơn.".into(),
            },
        );

        accept(&paths, "01A", &proposal.id).expect("accept");
        let body = std::fs::read_to_string(paths.meetings().join("01A.md")).unwrap();
        assert!(body.contains("Mới và đúng hơn."), "{body}");
        assert!(!body.contains("Cũ."));
    }

    #[test]
    fn a_proposal_with_no_action_is_refused_at_the_point_it_is_made() {
        let (_d, paths) = vault("");
        let err = say(
            &paths,
            Saying {
                note: "01A",
                kind: Kind::Proposal,
                author: "agent",
                body: "làm gì đó đi",
                anchor: Anchor::Note,
                action: None,
                at: "2026-08-10T10:00:00+07:00",
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("must carry the action"), "{err}");
    }

    #[test]
    fn a_comment_needs_no_action_and_cannot_be_accepted() {
        let (_d, paths) = vault("");
        let comment = say(
            &paths,
            Saying {
                note: "01A",
                kind: Kind::Comment,
                author: "Ngọc",
                body: "Chỗ này nói khác",
                anchor: Anchor::Segment { seq: 12 },
                action: None,
                at: "2026-08-10T09:00:00+07:00",
            },
        )
        .expect("comment");

        assert!(accept(&paths, "01A", &comment.id).is_err());
        assert!(pending(&paths).expect("pending").is_empty());
    }

    #[test]
    fn an_empty_comment_is_refused() {
        let (_d, paths) = vault("");
        assert!(
            say(
                &paths,
                Saying {
                    note: "01A",
                    kind: Kind::Comment,
                    author: "Ngọc",
                    body: "   ",
                    anchor: Anchor::Note,
                    action: None,
                    at: "2026-08-10T09:00:00+07:00",
                },
            )
            .is_err()
        );
    }

    #[test]
    fn pending_gathers_decisions_across_the_vault_oldest_first() {
        let (_d, paths) = vault("");
        std::fs::write(
            paths.meetings().join("01B.md"),
            "---\nid: 01B\ndate: 2026-08-09T09:00:00+07:00\nduration: 60\n\
             participants: []\ntags: []\n---\n# Họp khác\n",
        )
        .unwrap();

        say(
            &paths,
            Saying {
                note: "01B",
                kind: Kind::Proposal,
                author: "agent",
                body: "sớm hơn",
                anchor: Anchor::Note,
                action: Some(Action::CreateTask {
                    text: "A".into(),
                    owner: None,
                    due: None,
                }),
                at: "2026-08-10T08:00:00+07:00",
            },
        )
        .expect("b");
        propose(
            &paths,
            Action::CreateTask {
                text: "B".into(),
                owner: None,
                due: None,
            },
        );

        let waiting = pending(&paths).expect("pending");
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0].0, "01B", "oldest first: {waiting:?}");
    }
}
