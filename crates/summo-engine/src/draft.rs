//! The summary before it is a summary.
//!
//! A meeting ends, the agent writes a draft, and the user reads it. Most of the time they confirm
//! it. Sometimes a sentence is wrong, so they select it and say what they want instead, or they
//! argue with it in the chat panel until it is right. Then they confirm, and *that* is when it
//! becomes part of the note.
//!
//! ```text
//!   stop ──► draft ──► read ──┬─ ok ─────────────────► confirm ──► written into the note
//!                             ├─ select + prompt ──┐
//!                             └─ chat ─────────────┴──► new draft ──► read again
//! ```
//!
//! **Nothing reaches the vault until confirm.** A draft lives in `~/.summo/drafts/`, outside the
//! vault entirely, so a summary the user never looked at cannot end up in a file they sync, back up
//! or open in Obsidian. It also means "discard" is a delete rather than an undo.
//!
//! **Confirm is one action for the whole draft**, not a decision per section. Reviewing a summary is
//! reading it once and saying yes; making somebody approve four sections separately turns a
//! two-second job into four.
//!
//! The two ways of revising are genuinely different, and the difference is the interesting part.
//! Selecting a passage says *where*, so the model is asked to return only that passage and
//! everything else stays byte-identical. Chatting does not say where, so the model returns the whole
//! draft and may touch anything. The first is precise and cheap; the second is powerful and has to
//! be re-read.

use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, paths::Paths};
use summo_llm::{LlmClient, prompt};
use summo_vault::{meeting::MeetingDoc, template::Templates};

/// One `##` section of a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub heading: String,
    pub body: String,
}

/// A turn in the conversation about the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// `you` or `agent`.
    pub role: String,
    pub text: String,
}

/// A summary waiting to be confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub meeting: MeetingId,
    pub template: String,
    pub sections: Vec<Section>,
    /// The refinement conversation, so reopening the panel shows what was already asked.
    #[serde(default)]
    pub turns: Vec<Turn>,
    /// How many times it has been revised. Shown so a user can tell a fresh draft from a worked one.
    #[serde(default)]
    pub revisions: u32,
}

impl Draft {
    /// The draft as Markdown, which is both what the model is shown and what gets written.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        self.sections
            .iter()
            .map(|s| format!("## {}\n{}", s.heading, s.body.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Parse a model's Markdown back into sections.
    #[must_use]
    pub fn sections_from(markdown: &str) -> Vec<Section> {
        let mut out = Vec::new();
        let mut heading: Option<String> = None;
        let mut buffer = String::new();

        for line in markdown.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("## ") {
                if let Some(previous) = heading.take() {
                    out.push(Section {
                        heading: previous,
                        body: buffer.trim().to_string(),
                    });
                }
                heading = Some(rest.trim().to_string());
                buffer.clear();
            } else if heading.is_some() {
                buffer.push_str(line);
                buffer.push('\n');
            }
        }
        if let Some(previous) = heading {
            out.push(Section {
                heading: previous,
                body: buffer.trim().to_string(),
            });
        }
        out.retain(|s| !s.body.is_empty());
        out
    }

    fn section_mut(&mut self, heading: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.heading == heading)
    }
}

fn path_for(paths: &Paths, meeting: &MeetingId) -> std::path::PathBuf {
    // Outside the vault: an unreviewed summary must not appear in the user's synced notes.
    paths
        .root()
        .join("drafts")
        .join(format!("{}.json", meeting.as_str()))
}

pub fn load(paths: &Paths, meeting: &MeetingId) -> Result<Option<Draft>> {
    let path = path_for(paths, meeting);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::Other(format!("cannot parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&path, e)),
    }
}

pub fn save(paths: &Paths, draft: &Draft) -> Result<()> {
    let path = path_for(paths, &draft.meeting);
    summo_vault::write::write_atomically(&path, serde_json::to_vec_pretty(draft)?.as_slice())
}

/// Throw a draft away without writing anything.
pub fn discard(paths: &Paths, meeting: &MeetingId) -> Result<bool> {
    let path = path_for(paths, meeting);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::io(&path, e)),
    }
}

/// Write the draft into the meeting's note, and delete the draft.
///
/// This is the only path by which a generated summary reaches the vault.
pub fn confirm(paths: &Paths, meeting: &MeetingId) -> Result<Vec<String>> {
    let draft = load(paths, meeting)?
        .ok_or_else(|| Error::Other(format!("no draft for meeting {meeting}")))?;
    if draft.sections.is_empty() {
        return Err(Error::Other("the draft is empty".into()));
    }

    let (path, markdown) = read_note(paths, meeting)?;
    let mut doc = MeetingDoc::parse(&markdown)?;
    let mut written = Vec::new();
    for section in &draft.sections {
        doc.set_section(&section.heading, section.body.trim());
        written.push(section.heading.clone());
    }
    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;

    // Only after the note is safely on disk. A crash between the two leaves the draft, which the
    // user can confirm again — the reverse would lose the summary.
    discard(paths, meeting)?;
    Ok(written)
}

/// Produce the first draft.
pub async fn generate(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    template_id: Option<&str>,
) -> Result<Draft> {
    let (_, markdown) = read_note(paths, meeting)?;
    let doc = MeetingDoc::parse(&markdown)?;
    let transcript = prompt::render_transcript(&doc.transcript);
    if transcript.chars().count() < 400 {
        return Err(Error::Other(
            "the transcript is too short to summarise".into(),
        ));
    }

    let templates = Templates::load_or_seed(&paths.templates())?;
    let template = match template_id {
        Some(id) => templates
            .get(id)
            .ok_or_else(|| Error::Other(format!("no template with id {id}")))?,
        None => templates
            .best_for(&doc.title, &doc.frontmatter.tags)
            .ok_or_else(|| Error::Other("no summary templates are installed".into()))?,
    };

    let language = language(paths, &template.language);
    let messages = prompt::summarize_with(&transcript, &template.instructions(), &language);
    let response = client.complete(&messages).await?;

    let sections = Draft::sections_from(&response);
    if sections.is_empty() {
        return Err(Error::Other(
            "the model returned nothing that looks like the requested sections".into(),
        ));
    }

    let draft = Draft {
        meeting: meeting.clone(),
        template: template.id.clone(),
        sections,
        turns: Vec::new(),
        revisions: 0,
    };
    save(paths, &draft)?;
    Ok(draft)
}

/// Rewrite one selected passage, leaving the rest of the draft untouched.
///
/// `selection` must appear in the named section verbatim; if the draft moved on since the user
/// selected it, that is a stale request and refusing beats rewriting the wrong span.
pub async fn refine(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    heading: &str,
    selection: &str,
    instruction: &str,
) -> Result<Draft> {
    let mut draft = load(paths, meeting)?
        .ok_or_else(|| Error::Other(format!("no draft for meeting {meeting}")))?;

    let language = language(paths, "");
    let section = draft
        .section_mut(heading)
        .ok_or_else(|| Error::Other(format!("the draft has no section `{heading}`")))?;

    let Some(at) = section.body.find(selection) else {
        return Err(Error::Other(
            "that passage is no longer in the draft; it may have changed since you selected it"
                .into(),
        ));
    };

    let messages = prompt::revise_selection(&section.body, selection, instruction, &language);
    let replacement = client.complete(&messages).await?;
    let replacement = strip_fence(&replacement);

    // Splice: everything outside the selection is byte-identical afterwards.
    let mut body = String::with_capacity(section.body.len() + replacement.len());
    body.push_str(&section.body[..at]);
    body.push_str(replacement.trim());
    body.push_str(&section.body[at + selection.len()..]);
    section.body = body;

    draft.revisions += 1;
    draft.turns.push(Turn {
        role: "you".into(),
        text: format!("Sửa “{}”: {instruction}", shorten(selection)),
    });
    draft.turns.push(Turn {
        role: "agent".into(),
        text: format!("Đã sửa trong mục {heading}."),
    });
    save(paths, &draft)?;
    Ok(draft)
}

/// Revise the whole draft in response to a chat message.
pub async fn chat(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    message: &str,
) -> Result<Draft> {
    let mut draft = load(paths, meeting)?
        .ok_or_else(|| Error::Other(format!("no draft for meeting {meeting}")))?;

    let (_, markdown) = read_note(paths, meeting)?;
    let doc = MeetingDoc::parse(&markdown)?;
    let transcript = prompt::render_transcript(&doc.transcript);
    let language = language(paths, "");

    let messages = prompt::revise_draft(&draft.to_markdown(), message, &transcript, &language);
    let response = client.complete(&messages).await?;

    let sections = Draft::sections_from(&response);
    if sections.is_empty() {
        // Better to keep the draft the user had than to replace it with nothing.
        return Err(Error::Other(
            "the model's reply did not contain a draft; the previous one is unchanged".into(),
        ));
    }

    draft.sections = sections;
    draft.revisions += 1;
    draft.turns.push(Turn {
        role: "you".into(),
        text: message.trim().to_string(),
    });
    draft.turns.push(Turn {
        role: "agent".into(),
        text: "Đã cập nhật bản nháp.".into(),
    });
    save(paths, &draft)?;
    Ok(draft)
}

fn read_note(paths: &Paths, meeting: &MeetingId) -> Result<(std::path::PathBuf, String)> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;
    let entry = index
        .entries()
        .iter()
        .find(|e| &e.id == meeting)
        .ok_or_else(|| Error::Other(format!("no meeting with id {meeting}")))?;
    let path = vault.join(&entry.path);
    let markdown = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    Ok((path, markdown))
}

fn language(paths: &Paths, from_template: &str) -> String {
    if !from_template.is_empty() {
        return from_template.to_string();
    }
    summo_core::settings::Settings::load(&paths.settings())
        .ok()
        .map(|s| s.llm.language)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "the language of the transcript".into())
}

/// Models wrap replacements in fences often enough that not stripping them is a visible bug.
fn strip_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest.split_once('\n').map_or(rest, |(_, body)| body);
    rest.trim_end().strip_suffix("```").unwrap_or(rest).trim()
}

/// A passage, short enough to quote back in the conversation.
fn shorten(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 40 {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(40).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Paths) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(
            paths.meetings().join("01A.md"),
            "---\nid: 01A\ndate: 2026-08-10T09:00:00+07:00\nduration: 600\n\
             participants: []\ntags: []\n---\n# Họp\n\n## Tóm tắt\nBản cũ trong note.\n",
        )
        .unwrap();
        (dir, paths)
    }

    fn draft() -> Draft {
        Draft {
            meeting: MeetingId::from("01A".to_string()),
            template: "standard".into(),
            sections: vec![
                Section {
                    heading: "Tóm tắt".into(),
                    body: "Câu một. Câu hai. Câu ba.".into(),
                },
                Section {
                    heading: "Quyết định".into(),
                    body: "- Dùng Rust".into(),
                },
            ],
            turns: Vec::new(),
            revisions: 0,
        }
    }

    #[test]
    fn a_draft_lives_outside_the_vault() {
        let (_d, paths) = vault();
        save(&paths, &draft()).expect("save");
        let inside = paths.vault().join("drafts");
        assert!(!inside.exists(), "an unreviewed summary must not be in the user's notes");
        assert!(paths.root().join("drafts").join("01A.json").exists());
    }

    #[test]
    fn a_meeting_without_a_draft_has_none() {
        let (_d, paths) = vault();
        assert!(load(&paths, &MeetingId::from("01A".to_string())).unwrap().is_none());
    }

    #[test]
    fn a_draft_survives_a_round_trip() {
        let (_d, paths) = vault();
        let original = draft();
        save(&paths, &original).expect("save");
        let back = load(&paths, &original.meeting).expect("load").expect("some");
        assert_eq!(back, original);
    }

    /// The whole point: reading the draft changes nothing until confirm.
    #[test]
    fn a_draft_does_not_touch_the_note_until_confirmed() {
        let (_d, paths) = vault();
        save(&paths, &draft()).expect("save");

        let body = std::fs::read_to_string(paths.meetings().join("01A.md")).unwrap();
        assert!(body.contains("Bản cũ trong note."));
        assert!(!body.contains("Câu một."));
    }

    #[test]
    fn confirming_writes_every_section_and_removes_the_draft() {
        let (_d, paths) = vault();
        let d = draft();
        save(&paths, &d).expect("save");

        let written = confirm(&paths, &d.meeting).expect("confirm");
        assert_eq!(written, vec!["Tóm tắt", "Quyết định"]);

        let body = std::fs::read_to_string(paths.meetings().join("01A.md")).unwrap();
        assert!(body.contains("Câu một."), "{body}");
        assert!(body.contains("Dùng Rust"));
        assert!(!body.contains("Bản cũ trong note."), "the old section should be replaced");

        assert!(load(&paths, &d.meeting).unwrap().is_none(), "the draft is gone");
    }

    #[test]
    fn confirming_twice_is_an_error_not_a_second_write() {
        let (_d, paths) = vault();
        let d = draft();
        save(&paths, &d).expect("save");
        confirm(&paths, &d.meeting).expect("first");
        assert!(confirm(&paths, &d.meeting).is_err());
    }

    #[test]
    fn discarding_leaves_the_note_alone() {
        let (_d, paths) = vault();
        let d = draft();
        save(&paths, &d).expect("save");

        assert!(discard(&paths, &d.meeting).expect("discard"));
        assert!(!discard(&paths, &d.meeting).expect("again"), "already gone");

        let body = std::fs::read_to_string(paths.meetings().join("01A.md")).unwrap();
        assert!(body.contains("Bản cũ trong note."));
    }

    #[test]
    fn markdown_round_trips_through_sections() {
        let d = draft();
        let parsed = Draft::sections_from(&d.to_markdown());
        assert_eq!(parsed, d.sections);
    }

    #[test]
    fn a_fenced_reply_is_still_parsed() {
        let sections = Draft::sections_from("```markdown\n## Tóm tắt\nNội dung.\n```");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].body, "Nội dung.");
    }

    #[test]
    fn an_empty_section_is_dropped_rather_than_written_blank() {
        let sections = Draft::sections_from("## A\n\n## B\ncó nội dung\n");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "B");
    }

    #[test]
    fn a_fence_around_a_replacement_is_stripped() {
        assert_eq!(strip_fence("```\nxin chào\n```"), "xin chào");
        assert_eq!(strip_fence("```markdown\nxin chào\n```"), "xin chào");
        assert_eq!(strip_fence("  xin chào  "), "xin chào");
    }

    #[test]
    fn a_quoted_passage_is_shortened_for_the_conversation() {
        assert_eq!(shorten("ngắn"), "ngắn");
        let long = "ề".repeat(60);
        let short = shorten(&long);
        assert!(short.ends_with('…'));
        assert_eq!(short.chars().count(), 41);
    }
}
