//! Talking about a note without editing it.
//!
//! This is the collaboration layer, and it exists because of one rule: **the agent proposes, the
//! human decides.** An assistant that silently rewrites your notes is not a collaborator, it is a
//! liability — you stop trusting anything in the file because you cannot tell what you wrote and
//! what it wrote.
//!
//! Humans and the agent use the same mechanism, which is what makes it a conversation rather than a
//! notification tray:
//!
//! | From | Kind | Example |
//! |---|---|---|
//! | a person | [`Kind::Comment`] | "Chỗ này Ngọc nói khác" |
//! | the agent | [`Kind::Proposal`] | "Thêm việc `@binh gửi báo giá`" + accept / dismiss |
//! | the agent | [`Kind::Question`] | "Ai chịu trách nhiệm việc này?" |
//!
//! A proposal carries the [`Action`] it would perform, so accepting is one click and the user can
//! see exactly what they are agreeing to before they agree to it. Nothing is applied until they do.
//!
//! Annotations live beside the note rather than inside it — `~/.summo/vault/annotations/<id>.md` —
//! so the note stays clean to read in Obsidian while the conversation about it stays greppable.

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};

/// Who or what is talking, and in what register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A person, saying something to other people.
    Comment,
    /// The agent, offering to do something. Carries an [`Action`].
    Proposal,
    /// The agent, needing an answer it cannot work out. Blocks nothing.
    Question,
}

/// Where an annotation is attached.
///
/// A note-level annotation is about the whole thing; a segment anchor pins it to one utterance,
/// which is what makes "Ngọc said something different at 12:04" reviewable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "kebab-case")]
pub enum Anchor {
    Note,
    /// A transcript utterance, by sequence number.
    Segment { seq: u64 },
    /// A `##` heading in the note.
    Section { heading: String },
    /// A task, by id.
    Task { id: String },
}

/// Something the agent is offering to do.
///
/// Deliberately an enum rather than free-form text: a proposal the user cannot see the shape of is
/// a proposal they cannot consent to, and a string the agent writes could say anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "kebab-case")]
pub enum Action {
    /// Add a task to this note.
    CreateTask {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        due: Option<String>,
    },
    /// Change one that exists.
    UpdateTask {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
    },
    /// Rewrite a section of the summary.
    SetSection { heading: String, body: String },
}

impl Action {
    /// One line describing what accepting would do, for the button's label and the audit trail.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::CreateTask { text, owner, due } => {
                let mut out = format!("Thêm việc `{text}`");
                if let Some(owner) = owner {
                    out.push_str(&format!(" cho @{owner}"));
                }
                if let Some(due) = due {
                    out.push_str(&format!(", hạn {due}"));
                }
                out
            }
            Self::UpdateTask { id, status, owner } => {
                let mut out = format!("Sửa việc {id}");
                if let Some(status) = status {
                    out.push_str(&format!(" → {status}"));
                }
                if let Some(owner) = owner {
                    out.push_str(&format!(", giao cho @{owner}"));
                }
                out
            }
            Self::SetSection { heading, .. } => format!("Viết lại mục `{heading}`"),
        }
    }
}

/// Whether a proposal is still waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Resolution {
    Open,
    Accepted,
    Dismissed,
}

/// One thing said about a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub kind: Kind,
    /// Display name, or `agent`.
    pub author: String,
    /// ISO-8601, in the offset it was written in.
    pub at: String,
    pub body: String,
    #[serde(default = "note_anchor")]
    pub anchor: Anchor,
    /// Present on a proposal, absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    #[serde(default = "open")]
    pub resolution: Resolution,
    /// Emoji, and who reacted. Reactions are on comments, not proposals — a proposal is answered
    /// by accepting it, and offering a thumbs-up as well would make the state ambiguous.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub by: Vec<String>,
}

fn note_anchor() -> Anchor {
    Anchor::Note
}

fn open() -> Resolution {
    Resolution::Open
}

impl Annotation {
    /// Whether this is a proposal still waiting on a decision.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.kind == Kind::Proposal && self.resolution == Resolution::Open
    }

    /// Whether the agent wrote it.
    #[must_use]
    pub fn from_agent(&self) -> bool {
        self.author == "agent"
    }
}

/// Every annotation on one note.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl Thread {
    /// Add a comment from a person.
    ///
    /// Written here rather than by the caller so an annotation always has an id and a timestamp:
    /// an entry with neither cannot be replied to, reacted to or resolved, and one written by hand
    /// eventually has neither.
    pub fn comment(&mut self, author: &str, body: &str, anchor: Anchor, at: String) -> Result<&Annotation> {
        let body = body.trim();
        if body.is_empty() {
            return Err(Error::msg("comment.empty", "bình luận không có nội dung"));
        }

        self.annotations.push(Annotation {
            id: summo_core::MeetingId::new().to_string(),
            kind: Kind::Comment,
            author: author.trim().to_string(),
            at,
            body: body.to_string(),
            anchor,
            action: None,
            resolution: Resolution::Open,
            reactions: Vec::new(),
        });
        Ok(self.annotations.last().expect("just pushed"))
    }

    /// Remove one. `false` when it was not there.
    ///
    /// Deleting rather than tombstoning: this is a Markdown file in the user's own vault, and a
    /// comment they deleted should not still be in it when they open it in Obsidian.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.annotations.len();
        self.annotations.retain(|a| a.id != id);
        before != self.annotations.len()
    }

    /// Proposals nobody has answered yet, oldest first — the agent's call to action.
    #[must_use]
    pub fn pending(&self) -> Vec<&Annotation> {
        let mut out: Vec<&Annotation> = self.annotations.iter().filter(|a| a.is_pending()).collect();
        out.sort_by(|a, b| a.at.cmp(&b.at));
        out
    }

    /// Everything attached to one anchor, in the order it was said.
    #[must_use]
    pub fn at(&self, anchor: &Anchor) -> Vec<&Annotation> {
        let mut out: Vec<&Annotation> = self
            .annotations
            .iter()
            .filter(|a| &a.anchor == anchor)
            .collect();
        out.sort_by(|a, b| a.at.cmp(&b.at));
        out
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.id == id)
    }

    /// Add a reaction, or remove it if that person already reacted with that emoji.
    ///
    /// Toggling rather than adding means the same gesture undoes itself, which is what every other
    /// product with reactions has taught people to expect.
    pub fn react(&mut self, id: &str, emoji: &str, by: &str) -> Result<()> {
        let annotation = self
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::Other(format!("no annotation with id {id}")))?;

        if let Some(reaction) = annotation.reactions.iter_mut().find(|r| r.emoji == emoji) {
            if let Some(at) = reaction.by.iter().position(|p| p == by) {
                reaction.by.remove(at);
            } else {
                reaction.by.push(by.to_string());
            }
        } else {
            annotation.reactions.push(Reaction {
                emoji: emoji.to_string(),
                by: vec![by.to_string()],
            });
        }
        // An emoji nobody is using any more is not a zero, it is gone.
        annotation.reactions.retain(|r| !r.by.is_empty());
        Ok(())
    }

    /// Mark a proposal as decided.
    pub fn resolve(&mut self, id: &str, resolution: Resolution) -> Result<Annotation> {
        let annotation = self
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::Other(format!("no annotation with id {id}")))?;

        if annotation.kind != Kind::Proposal {
            return Err(Error::Other(format!(
                "annotation {id} is a {:?}, which is not something to accept or dismiss",
                annotation.kind
            )));
        }
        if annotation.resolution != Resolution::Open {
            return Err(Error::Other(format!(
                "proposal {id} was already {:?}",
                annotation.resolution
            )));
        }
        annotation.resolution = resolution;
        Ok(annotation.clone())
    }
}

/// Where a note's conversation lives.
#[must_use]
pub fn path_for(paths: &Paths, note: &str) -> std::path::PathBuf {
    paths
        .vault()
        .join("annotations")
        .join(format!("{note}.json"))
}

/// Read a note's thread, treating a missing file as an empty one.
pub fn load(paths: &Paths, note: &str) -> Result<Thread> {
    let path = path_for(paths, note);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| Error::Other(format!("cannot parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Thread::default()),
        Err(e) => Err(Error::io(&path, e)),
    }
}

pub fn save(paths: &Paths, note: &str, thread: &Thread) -> Result<()> {
    let path = path_for(paths, note);
    crate::write::write_atomically(&path, serde_json::to_vec_pretty(thread)?.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn annotation(id: &str, kind: Kind, author: &str, at: &str) -> Annotation {
        Annotation {
            id: id.into(),
            kind,
            author: author.into(),
            at: at.into(),
            body: "nội dung".into(),
            anchor: Anchor::Note,
            action: (kind == Kind::Proposal).then(|| Action::CreateTask {
                text: "Gửi báo giá".into(),
                owner: Some("binh".into()),
                due: None,
            }),
            resolution: Resolution::Open,
            reactions: Vec::new(),
        }
    }

    /// An entry with no id and no timestamp cannot be replied to, reacted to or resolved — so the
    /// thread writes both rather than trusting a caller to.
    #[test]
    fn a_comment_gets_an_id_and_keeps_what_was_written() {
        let mut thread = Thread::default();
        let added = thread
            .comment("Ngọc", "  Chốt vào thứ sáu  ", Anchor::Note, "2026-08-11T09:00:00+07:00".into())
            .unwrap()
            .clone();

        assert!(!added.id.is_empty());
        assert_eq!(added.body, "Chốt vào thứ sáu", "trimmed, not mangled");
        assert_eq!(added.kind, Kind::Comment);
        assert_eq!(added.resolution, Resolution::Open);
    }

    #[test]
    fn an_empty_comment_is_refused() {
        let mut thread = Thread::default();
        assert!(thread.comment("Ngọc", "   ", Anchor::Note, "t".into()).is_err());
        assert!(thread.annotations.is_empty());
    }

    #[test]
    fn a_comment_can_be_pinned_to_one_utterance() {
        let mut thread = Thread::default();
        thread
            .comment("Ngọc", "sai chỗ này", Anchor::Segment { seq: 12 }, "t".into())
            .unwrap();
        assert_eq!(thread.at(&Anchor::Segment { seq: 12 }).len(), 1);
        assert!(thread.at(&Anchor::Note).is_empty());
    }

    /// This is a Markdown file in the user's own vault. A comment they deleted must not still be in
    /// it when they open it in Obsidian.
    #[test]
    fn removing_a_comment_takes_it_out_of_the_file() {
        let mut thread = Thread::default();
        let id = thread
            .comment("Ngọc", "xoá tôi", Anchor::Note, "t".into())
            .unwrap()
            .id
            .clone();

        assert!(thread.remove(&id));
        assert!(thread.annotations.is_empty());
        assert!(!thread.remove(&id), "removing twice is not an error");
    }

    #[test]
    fn two_comments_get_different_ids() {
        let mut thread = Thread::default();
        let a = thread.comment("x", "một", Anchor::Note, "t".into()).unwrap().id.clone();
        let b = thread.comment("x", "hai", Anchor::Note, "t".into()).unwrap().id.clone();
        assert_ne!(a, b);
    }

    fn thread() -> Thread {
        Thread {
            annotations: vec![
                annotation("A2", Kind::Proposal, "agent", "2026-08-10T10:00:00+07:00"),
                annotation("A1", Kind::Comment, "Ngọc", "2026-08-10T09:00:00+07:00"),
                annotation("A3", Kind::Question, "agent", "2026-08-10T11:00:00+07:00"),
            ],
        }
    }

    #[test]
    fn only_unanswered_proposals_are_pending() {
        let t = thread();
        let pending: Vec<&str> = t.pending().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(pending, vec!["A2"], "a comment and a question are not decisions to make");
    }

    #[test]
    fn a_decided_proposal_stops_being_pending() {
        let mut t = thread();
        t.resolve("A2", Resolution::Accepted).expect("resolve");
        assert!(t.pending().is_empty());
    }

    #[test]
    fn a_proposal_cannot_be_decided_twice() {
        let mut t = thread();
        t.resolve("A2", Resolution::Accepted).expect("first");
        let err = t.resolve("A2", Resolution::Dismissed).unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");
    }

    /// Accepting a comment is meaningless, and allowing it would make the state unreadable.
    #[test]
    fn only_a_proposal_can_be_accepted() {
        let mut t = thread();
        assert!(t.resolve("A1", Resolution::Accepted).is_err());
        assert!(t.resolve("A3", Resolution::Accepted).is_err());
    }

    #[test]
    fn resolving_something_that_is_not_there_is_an_error() {
        let mut t = thread();
        assert!(t.resolve("NOPE", Resolution::Accepted).is_err());
    }

    #[test]
    fn a_thread_is_ordered_by_when_it_was_said() {
        let mut t = thread();
        t.annotations.push(Annotation {
            anchor: Anchor::Segment { seq: 12 },
            ..annotation("A4", Kind::Comment, "Bình", "2026-08-10T08:00:00+07:00")
        });
        let ids: Vec<&str> = t.at(&Anchor::Note).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["A1", "A2", "A3"]);
    }

    #[test]
    fn an_anchor_selects_only_its_own_conversation() {
        let mut t = thread();
        t.annotations.push(Annotation {
            anchor: Anchor::Segment { seq: 12 },
            ..annotation("A4", Kind::Comment, "Bình", "2026-08-10T12:00:00+07:00")
        });
        assert_eq!(t.at(&Anchor::Segment { seq: 12 }).len(), 1);
        assert_eq!(t.at(&Anchor::Segment { seq: 99 }).len(), 0);
        assert_eq!(t.at(&Anchor::Note).len(), 3);
    }

    #[test]
    fn reacting_twice_with_the_same_emoji_takes_it_back() {
        let mut t = thread();
        t.react("A1", "❤️", "Ngọc").expect("add");
        assert_eq!(t.get("A1").unwrap().reactions.len(), 1);

        t.react("A1", "❤️", "Ngọc").expect("remove");
        assert!(
            t.get("A1").unwrap().reactions.is_empty(),
            "an emoji nobody uses is gone, not a zero"
        );
    }

    #[test]
    fn several_people_can_share_a_reaction() {
        let mut t = thread();
        t.react("A1", "👍", "Ngọc").expect("a");
        t.react("A1", "👍", "Bình").expect("b");
        let reactions = &t.get("A1").unwrap().reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].by, vec!["Ngọc", "Bình"]);
    }

    #[test]
    fn reacting_to_something_that_is_not_there_is_an_error() {
        assert!(thread().react("NOPE", "👍", "Ngọc").is_err());
    }

    /// The user has to be able to read what they are agreeing to before agreeing to it.
    #[test]
    fn an_action_describes_itself_in_words() {
        let create = Action::CreateTask {
            text: "Gửi báo giá".into(),
            owner: Some("binh".into()),
            due: Some("2026-08-12".into()),
        };
        let described = create.describe();
        assert!(described.contains("Gửi báo giá"), "{described}");
        assert!(described.contains("@binh"), "{described}");
        assert!(described.contains("2026-08-12"), "{described}");

        let update = Action::UpdateTask {
            id: "T1".into(),
            status: Some("done".into()),
            owner: None,
        };
        assert!(update.describe().contains("done"));

        let section = Action::SetSection {
            heading: "Tóm tắt".into(),
            body: "…".into(),
        };
        assert!(section.describe().contains("Tóm tắt"));
    }

    #[test]
    fn a_note_nobody_has_commented_on_has_an_empty_thread() {
        let dir = TempDir::new().unwrap();
        let thread = load(&Paths::at(dir.path()), "01A").expect("load");
        assert!(thread.annotations.is_empty());
    }

    #[test]
    fn a_thread_survives_a_round_trip() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let mut original = thread();
        original.react("A1", "🎉", "Ngọc").expect("react");
        original.resolve("A2", Resolution::Accepted).expect("resolve");

        save(&paths, "01A", &original).expect("save");
        let reloaded = load(&paths, "01A").expect("load");
        assert_eq!(reloaded, original);
    }

    #[test]
    fn a_corrupt_thread_is_reported_rather_than_silently_emptied() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let path = path_for(&paths, "01A");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        // Losing a conversation quietly is worse than refusing to open it.
        assert!(load(&paths, "01A").is_err());
    }

    #[test]
    fn a_proposal_round_trips_its_action() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        save(&paths, "01A", &thread()).expect("save");
        let reloaded = load(&paths, "01A").expect("load");
        let proposal = reloaded.get("A2").expect("the proposal");
        assert!(matches!(
            proposal.action,
            Some(Action::CreateTask { ref text, .. }) if text == "Gửi báo giá"
        ));
    }
}
