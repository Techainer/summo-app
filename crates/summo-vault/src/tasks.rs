//! Work that came out of a meeting, and what happened to it.
//!
//! Tasks are Markdown checkboxes in the vault, not rows in a database. That follows ADR 0002 for
//! the usual reason — the user's notes are the user's files — but it earns its place here for a
//! second one: a task list that only Summo can read is a task list nobody looks at. These render as
//! ordinary checkboxes in Obsidian, GitHub, or anything else that reads Markdown.
//!
//! ```markdown
//! ## Việc cần làm
//! - [ ] @ngoc Chốt spec API <!-- id:01J4 due:2026-08-12 status:doing -->
//! - [ ] @agent Tạo lịch cho mốc ra mắt <!-- id:01J5 status:running -->
//!   - [x] Quét ghi chú tìm mốc thời gian
//!   - [ ] Soạn sự kiện
//! ```
//!
//! Metadata rides in an HTML comment so the line still renders as a plain checkbox everywhere else.
//!
//! **Two kinds of task, one format.** `@agent` marks work the agent does itself; any other owner is
//! work for a person. The difference is not cosmetic — an agent task carries **steps**, which are
//! its own breakdown of the job and its own progress through it. That is the agent's board: a
//! person's task moves between three columns, an agent's task moves through a list it wrote itself.
//! Storing both as nested checkboxes means the user can read, edit, tick or delete either.

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

/// Headings a task list can live under, in either language.
///
/// Shared with [`crate::report`], which used to carry its own copy.
pub const TASK_HEADINGS: [&str; 4] = ["action items", "việc cần làm", "hành động", "todo"];

/// Where a task has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Todo,
    Doing,
    Done,
    /// Waiting on something or someone. Distinct from `todo`, which is merely unstarted.
    Blocked,
    /// An agent task that ran and failed. Needs a decision, not a retry loop.
    Failed,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(Self::Todo),
            "doing" | "running" => Some(Self::Doing),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done)
    }
}

/// One line of an agent's own breakdown of its task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub text: String,
    pub done: bool,
}

/// A task, as read from a Markdown file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub text: String,
    /// `@name` the line opens with. `agent` means the agent owns it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// The agent's own breakdown. Empty for a person's task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Vault-relative file the task lives in.
    pub file: String,
    /// Line number in that file, so an edit can be applied without re-parsing everything.
    pub line: usize,
}

impl Task {
    /// Whether the agent owns this task and is expected to run it.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        self.owner.as_deref() == Some("agent")
    }

    /// Progress through the agent's own steps, as a fraction. `None` when there are no steps.
    #[must_use]
    pub fn progress(&self) -> Option<f32> {
        if self.steps.is_empty() {
            return None;
        }
        let done = self.steps.iter().filter(|s| s.done).count();
        Some(done as f32 / self.steps.len() as f32)
    }

    /// Render back to the Markdown line it came from.
    fn to_line(&self) -> String {
        let box_ = if self.status.is_finished() { "x" } else { " " };
        let owner = self
            .owner
            .as_ref()
            .map(|o| format!("@{o} "))
            .unwrap_or_default();
        let due = self
            .due
            .as_ref()
            .map(|d| format!(" due:{d}"))
            .unwrap_or_default();
        format!(
            "- [{box_}] {owner}{} <!-- id:{}{due} status:{} -->",
            self.text,
            self.id,
            self.status.as_str()
        )
    }
}

/// Every task in one Markdown document.
///
/// `file` is stored on each task so a board assembled from many meetings can still write each
/// change back to the right place.
#[must_use]
pub fn parse(markdown: &str, file: &str) -> Vec<Task> {
    parse_scoped(markdown, file, Scope::ActionSections)
}

/// Where in a document checkboxes count as tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only under a heading that names actions.
    ///
    /// Right for a meeting note, where the summary is written by a model: a checkbox the model
    /// produced inside "Quyết định" is prose about a decision, not a task somebody agreed to own.
    ActionSections,
    /// Anywhere in the document.
    ///
    /// Right for a note a person typed, where every line is theirs. Somebody writing
    /// `- [ ] @ngoc gọi khách` in the middle of a paragraph means it, and requiring them to first
    /// invent a heading is the kind of rule that makes people keep their todos somewhere else.
    Everywhere,
}

/// Parse tasks, choosing how much of the document to look at.
#[must_use]
pub fn parse_scoped(markdown: &str, file: &str, scope: Scope) -> Vec<Task> {
    let mut out: Vec<Task> = Vec::new();
    let mut inside = scope == Scope::Everywhere;
    // Indentation of the most recent task, so a step can be recognised by being deeper than it
    // rather than by being indented at all.
    let mut parent_indent: Option<usize> = None;

    for (number, line) in markdown.lines().enumerate() {
        let trimmed = line.trim();

        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim().to_lowercase();
            // In `Everywhere`, a heading is just a heading — including the transcript's, which is
            // why that scope is only ever used on a document with no transcript.
            if scope == Scope::ActionSections {
                inside = TASK_HEADINGS.iter().any(|h| title.contains(h));
            }
            continue;
        }
        if !inside {
            continue;
        }

        let Some((done, text)) = checkbox(trimmed) else {
            continue;
        };

        // A step is a checkbox indented *deeper than the task above it* — that is how an agent
        // records its own breakdown. Measuring depth relative to the parent rather than against
        // zero matters because Markdown lets a whole list be indented; treating those as orphan
        // steps would swallow every task in the file.
        let indent = line.len() - line.trim_start().len();
        if let (Some(parent_at), Some(parent)) = (parent_indent, out.last_mut())
            && indent > parent_at
        {
            parent.steps.push(Step {
                text: strip_meta(text).trim().to_string(),
                done,
            });
            continue;
        }
        parent_indent = Some(indent);

        let (owner, rest) = split_owner(text);
        let meta = read_meta(text);
        out.push(Task {
            id: meta.id.unwrap_or_else(|| stable_id(file, number)),
            text: strip_meta(rest).trim().to_string(),
            owner,
            // An explicit `status:` wins; otherwise the checkbox itself is the source of truth,
            // because that is the part a user edits by hand.
            status: meta
                .status
                .unwrap_or(if done { Status::Done } else { Status::Todo }),
            due: meta.due,
            steps: Vec::new(),
            file: file.to_string(),
            line: number,
        });
    }
    out
}

/// Apply a change to one task, returning the rewritten document.
///
/// Rewrites only that task's line, leaving every other byte of the file untouched — these are the
/// user's notes, and a round-trip through a Markdown serialiser would reformat prose nobody asked
/// to have reformatted.
pub fn update(markdown: &str, task: &Task) -> Result<String> {
    let mut lines: Vec<&str> = markdown.lines().collect();
    let line = lines
        .get_mut(task.line)
        .ok_or_else(|| Error::Other(format!("line {} is past the end of {}", task.line, task.file)))?;

    let indent = &line[..line.len() - line.trim_start().len()];
    if checkbox(line.trim()).is_none() {
        return Err(Error::Other(format!(
            "line {} of {} is not a checkbox; the file changed underneath",
            task.line, task.file
        )));
    }

    let rewritten = format!("{indent}{}", task.to_line());
    let mut out = String::with_capacity(markdown.len() + 64);
    for (i, existing) in lines.iter().enumerate() {
        out.push_str(if i == task.line { &rewritten } else { existing });
        out.push('\n');
    }
    // Preserve whether the original ended with a newline.
    if !markdown.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

/// Append a task under the document's task heading, adding the heading if it is missing.
pub fn append(markdown: &str, task: &Task) -> String {
    let heading_at = markdown.lines().position(|line| {
        line.strip_prefix('#').is_some_and(|h| {
            let title = h.trim_start_matches('#').trim().to_lowercase();
            TASK_HEADINGS.iter().any(|k| title.contains(k))
        })
    });

    let Some(start) = heading_at else {
        let mut out = markdown.trim_end().to_string();
        out.push_str("\n\n## Việc cần làm\n");
        out.push_str(&task.to_line());
        out.push('\n');
        return out;
    };

    // Insert after the last line of that section, so tasks stay in the order they were added.
    let lines: Vec<&str> = markdown.lines().collect();
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim_start().starts_with("## ") {
            end = i;
            break;
        }
    }
    // Step back over blank lines so the new task sits against the list rather than after a gap.
    while end > start + 1 && lines.get(end - 1).is_some_and(|l| l.trim().is_empty()) {
        end -= 1;
    }

    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    out.insert(end, task.to_line());
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// `- [ ] text` or `* [x] text`, returning whether it is ticked and the rest.
fn checkbox(trimmed: &str) -> Option<(bool, &str)> {
    let rest = trimmed
        .strip_prefix("- [")
        .or_else(|| trimmed.strip_prefix("* ["))?;
    let (mark, rest) = rest.split_at_checked(1)?;
    let text = rest.strip_prefix(']')?;
    Some((mark.eq_ignore_ascii_case("x"), text.trim_start()))
}

/// A leading `@name`, and what follows it.
fn split_owner(text: &str) -> (Option<String>, &str) {
    let Some(rest) = text.strip_prefix('@') else {
        return (None, text);
    };
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    let (name, remainder) = rest.split_at(end);
    if name.is_empty() {
        return (None, text);
    }
    (Some(name.to_string()), remainder.trim_start())
}

#[derive(Default)]
struct Meta {
    id: Option<String>,
    status: Option<Status>,
    due: Option<String>,
}

fn read_meta(text: &str) -> Meta {
    let Some(start) = text.find("<!--") else {
        return Meta::default();
    };
    let Some(end) = text[start..].find("-->") else {
        return Meta::default();
    };
    let mut meta = Meta::default();
    for field in text[start + 4..start + end].split_whitespace() {
        match field.split_once(':') {
            Some(("id", value)) => meta.id = Some(value.to_string()),
            Some(("status", value)) => meta.status = Status::parse(value),
            Some(("due", value)) => meta.due = Some(value.to_string()),
            _ => {}
        }
    }
    meta
}

fn strip_meta(text: &str) -> &str {
    match text.find("<!--") {
        Some(at) => &text[..at],
        None => text,
    }
}

/// An id for a task written by hand, which has no `<!-- id: -->` yet.
///
/// Derived from where it sits rather than random, so reading the same file twice yields the same
/// id and a board does not duplicate rows. It stops being stable if the line moves, which is why
/// anything Summo writes carries an explicit id.
fn stable_id(file: &str, line: usize) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in file.bytes().chain(line.to_string().bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("t{hash:012x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A person typing `- [ ] @ngoc gọi khách` in the middle of a note means it. Requiring them to
    /// invent a heading first is the rule that makes people keep their todos somewhere else.
    #[test]
    fn in_a_note_a_checkbox_anywhere_is_a_task() {
        let markdown = "# Ý tưởng\n\nMột dòng bất kỳ.\n- [ ] @ngoc Gọi khách\n";
        assert!(parse(markdown, "notes/x.md").is_empty(), "not in the default scope");

        let found = parse_scoped(markdown, "notes/x.md", Scope::Everywhere);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner.as_deref(), Some("ngoc"));
    }

    /// The reason the default is narrow: a model writing a summary produces checkboxes inside
    /// sections that are prose, and none of those are work anybody agreed to own.
    #[test]
    fn in_a_meeting_only_the_actions_section_counts() {
        let markdown = "# Họp\n\n## Quyết định\n- [ ] có vẻ như là một gạch đầu dòng\n\
## Việc cần làm\n- [ ] @ngoc Chốt spec\n";
        let found = parse(markdown, "meetings/x.md");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text.trim(), "Chốt spec");
    }

    #[test]
    fn the_everywhere_scope_still_reads_steps_under_their_parent() {
        let markdown = "- [ ] @agent Tạo lịch\n  - [x] Quét ghi chú\n  - [ ] Đăng lên lịch\n";
        let found = parse_scoped(markdown, "notes/x.md", Scope::Everywhere);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].steps.len(), 2);
    }

    const DOC: &str = "\
# Họp tuần
## Tóm tắt
Không phải việc.
- [ ] dòng này ngoài mục việc

## Việc cần làm
- [ ] @ngoc Chốt spec API <!-- id:01J4 due:2026-08-12 status:doing -->
- [x] Gửi biên bản <!-- id:01J5 -->
- [ ] @agent Tạo lịch cho mốc ra mắt <!-- id:01J6 status:running -->
  - [x] Quét ghi chú tìm mốc
  - [ ] Soạn sự kiện
";

    #[test]
    fn only_checkboxes_under_a_task_heading_count() {
        let tasks = parse(DOC, "m.md");
        assert_eq!(tasks.len(), 3, "{:?}", tasks.iter().map(|t| &t.text).collect::<Vec<_>>());
        assert!(tasks.iter().all(|t| !t.text.contains("ngoài mục việc")));
    }

    #[test]
    fn an_owner_and_metadata_are_read_off_the_line() {
        let task = &parse(DOC, "m.md")[0];
        assert_eq!(task.owner.as_deref(), Some("ngoc"));
        assert_eq!(task.text, "Chốt spec API");
        assert_eq!(task.status, Status::Doing);
        assert_eq!(task.due.as_deref(), Some("2026-08-12"));
        assert_eq!(task.id, "01J4");
    }

    /// The checkbox is what a user edits by hand, so it decides when no status is written.
    #[test]
    fn a_ticked_box_without_a_status_is_done() {
        let task = &parse(DOC, "m.md")[1];
        assert_eq!(task.status, Status::Done);
        assert!(task.owner.is_none());
    }

    #[test]
    fn indented_checkboxes_are_the_agents_own_steps() {
        let task = &parse(DOC, "m.md")[2];
        assert!(task.is_agent());
        assert_eq!(task.steps.len(), 2, "not four separate tasks");
        assert_eq!(task.steps[0].text, "Quét ghi chú tìm mốc");
        assert!(task.steps[0].done);
        assert!(!task.steps[1].done);
    }

    #[test]
    fn progress_counts_the_agents_steps() {
        let tasks = parse(DOC, "m.md");
        assert_eq!(tasks[2].progress(), Some(0.5));
        assert_eq!(tasks[0].progress(), None, "a person's task has no steps");
    }

    #[test]
    fn a_task_written_by_hand_gets_a_stable_id() {
        let doc = "## Việc cần làm\n- [ ] viết tay\n";
        let first = parse(doc, "m.md")[0].id.clone();
        let second = parse(doc, "m.md")[0].id.clone();
        assert_eq!(first, second, "reading twice must not produce two rows");
        assert_ne!(first, parse(doc, "khac.md")[0].id, "different files, different ids");
    }

    #[test]
    fn updating_rewrites_only_that_line() {
        let mut task = parse(DOC, "m.md")[0].clone();
        task.status = Status::Done;
        let out = update(DOC, &task).expect("update");

        assert!(out.contains("- [x] @ngoc Chốt spec API"), "{out}");
        assert!(out.contains("status:done"));
        // Everything else survives byte for byte.
        assert!(out.contains("Không phải việc."));
        assert!(out.contains("- [ ] dòng này ngoài mục việc"));
        assert!(out.contains("  - [x] Quét ghi chú tìm mốc"), "steps kept their indent");
    }

    #[test]
    fn updating_preserves_the_owner_and_due_date() {
        let mut task = parse(DOC, "m.md")[0].clone();
        task.status = Status::Blocked;
        let out = update(DOC, &task).expect("update");
        let reparsed = &parse(&out, "m.md")[0];
        assert_eq!(reparsed.owner.as_deref(), Some("ngoc"));
        assert_eq!(reparsed.due.as_deref(), Some("2026-08-12"));
        assert_eq!(reparsed.status, Status::Blocked);
    }

    #[test]
    fn a_round_trip_is_stable() {
        let tasks = parse(DOC, "m.md");
        let mut doc = DOC.to_string();
        for task in &tasks {
            doc = update(&doc, task).expect("update");
        }
        assert_eq!(parse(&doc, "m.md"), tasks, "rewriting unchanged tasks changed them");
    }

    /// The file is the user's; it can change underneath a board that was rendered a minute ago.
    #[test]
    fn updating_a_line_that_is_no_longer_a_checkbox_is_refused() {
        let mut task = parse(DOC, "m.md")[0].clone();
        task.line = 1; // "## Tóm tắt"
        assert!(update(DOC, &task).is_err());
    }

    #[test]
    fn updating_past_the_end_is_refused() {
        let mut task = parse(DOC, "m.md")[0].clone();
        task.line = 9_999;
        assert!(update(DOC, &task).is_err());
    }

    #[test]
    fn appending_adds_to_the_existing_section() {
        let task = Task {
            id: "01NEW".into(),
            text: "Việc mới".into(),
            owner: Some("binh".into()),
            status: Status::Todo,
            due: None,
            steps: Vec::new(),
            file: "m.md".into(),
            line: 0,
        };
        let out = append(DOC, &task);
        let tasks = parse(&out, "m.md");
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[3].text, "Việc mới");
        assert_eq!(tasks[3].owner.as_deref(), Some("binh"));
        // It must land inside the section, not after the whole document.
        assert!(!out.trim_end().ends_with("Soạn sự kiện"), "{out}");
    }

    #[test]
    fn appending_to_a_document_without_a_section_creates_one() {
        let task = Task {
            id: "01NEW".into(),
            text: "Việc đầu tiên".into(),
            owner: None,
            status: Status::Todo,
            due: None,
            steps: Vec::new(),
            file: "m.md".into(),
            line: 0,
        };
        let out = append("# Họp\nChỉ có văn xuôi.\n", &task);
        assert!(out.contains("## Việc cần làm"), "{out}");
        assert_eq!(parse(&out, "m.md").len(), 1);
    }

    #[test]
    fn an_english_heading_works_too() {
        let doc = "## Action items\n- [ ] @ngoc do the thing\n";
        assert_eq!(parse(doc, "m.md").len(), 1);
    }

    #[test]
    fn a_heading_after_the_list_closes_it() {
        let doc = "## Việc cần làm\n- [ ] thật\n## Transcript\n- [ ] giả\n";
        let tasks = parse(doc, "m.md");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "thật");
    }

    #[test]
    fn an_at_sign_with_no_name_is_not_an_owner() {
        let doc = "## Việc cần làm\n- [ ] @ trống\n";
        assert!(parse(doc, "m.md")[0].owner.is_none());
    }

    #[test]
    fn a_status_the_build_does_not_know_falls_back_to_the_checkbox() {
        let doc = "## Việc cần làm\n- [x] x <!-- id:1 status:teleported -->\n";
        assert_eq!(parse(doc, "m.md")[0].status, Status::Done);
    }

    /// Markdown lets a list be indented. Reading those as orphan steps would lose every task.
    #[test]
    fn an_indented_list_with_no_parent_is_still_a_list_of_tasks() {
        let doc = "## Việc cần làm\n  - [ ] một <!-- id:A -->\n  - [ ] hai <!-- id:B -->\n";
        let tasks = parse(doc, "m.md");
        assert_eq!(tasks.len(), 2, "{tasks:?}");
        assert!(tasks.iter().all(|t| t.steps.is_empty()));
    }

    #[test]
    fn running_is_read_as_doing() {
        // The agent writes `running`; the board shows three columns and needs it to land in one.
        let doc = "## Việc cần làm\n- [ ] x <!-- id:1 status:running -->\n";
        assert_eq!(parse(doc, "m.md")[0].status, Status::Doing);
    }
}
