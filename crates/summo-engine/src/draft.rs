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
//! **The draft goes straight into the note, marked.** `## Tóm tắt <!-- summo:draft -->` — an HTML
//! comment, so it vanishes when the Markdown is rendered and the note reads as finished in
//! Obsidian, while Summo can still tint the parts nobody has approved. Confirming removes the mark;
//! that is the whole gesture.
//!
//! Keeping the draft in a separate file was the first attempt and it is worse in a way that only
//! shows up in use: a note whose summary lives somewhere else *looks like a note with no summary*,
//! so the user opens it, sees nothing, and wonders whether the app worked. Marking beats hiding.
//!
//! Only the conversation is kept beside the note, in `~/.summo/drafts/<id>.json` — reopening the
//! panel should show what was already asked, and that is not something to leave in the user's
//! prose.
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
///
/// Assembled rather than stored: `sections` are read out of the note, and only `turns` and
/// `revisions` come from the sidecar. That way the text has exactly one home, and editing the note
/// by hand cannot leave the draft disagreeing with the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub meeting: MeetingId,
    pub template: String,
    /// The unapproved sections, in document order.
    pub sections: Vec<Section>,
    /// The refinement conversation, so reopening the panel shows what was already asked.
    #[serde(default)]
    pub turns: Vec<Turn>,
    /// How many times it has been revised. Shown so a user can tell a fresh draft from a worked one.
    #[serde(default)]
    pub revisions: u32,
}

/// What is kept beside the note: the conversation, never the prose.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Sidecar {
    #[serde(default)]
    template: String,
    #[serde(default)]
    turns: Vec<Turn>,
    #[serde(default)]
    revisions: u32,
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
    // Outside the vault, and holding only the conversation. The prose lives in the note.
    paths
        .root()
        .join("drafts")
        .join(format!("{}.json", meeting.as_str()))
}

fn read_sidecar(paths: &Paths, meeting: &MeetingId) -> Sidecar {
    // A missing or unreadable conversation costs the history panel, not the draft.
    std::fs::read_to_string(path_for(paths, meeting))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_sidecar(paths: &Paths, meeting: &MeetingId, sidecar: &Sidecar) -> Result<()> {
    let path = path_for(paths, meeting);
    summo_vault::write::write_atomically(&path, serde_json::to_vec_pretty(sidecar)?.as_slice())
}

fn clear_sidecar(paths: &Paths, meeting: &MeetingId) {
    std::fs::remove_file(path_for(paths, meeting)).ok();
}

/// The unapproved summary of a meeting, if there is one.
pub fn load(paths: &Paths, meeting: &MeetingId) -> Result<Option<Draft>> {
    let (_, doc) = read_note(paths, meeting)?;

    let sections: Vec<Section> = doc
        .sections
        .iter()
        .filter(|s| summo_vault::pending::is_draft(&s.heading))
        .map(|s| Section {
            heading: summo_vault::pending::strip(&s.heading).to_string(),
            body: s.body.clone(),
        })
        .collect();

    if sections.is_empty() {
        return Ok(None);
    }
    let sidecar = read_sidecar(paths, meeting);
    Ok(Some(Draft {
        meeting: meeting.clone(),
        template: sidecar.template,
        sections,
        turns: sidecar.turns,
        revisions: sidecar.revisions,
    }))
}

/// Remove every unapproved section, leaving anything a human wrote.
pub fn discard(paths: &Paths, meeting: &MeetingId) -> Result<bool> {
    let (path, mut doc) = read_note(paths, meeting)?;

    let headings = summo_vault::pending::in_document(&doc);
    if headings.is_empty() {
        clear_sidecar(paths, meeting);
        return Ok(false);
    }
    for heading in &headings {
        summo_vault::pending::reject(&mut doc, heading);
    }
    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;
    clear_sidecar(paths, meeting);
    Ok(true)
}

/// Approve the draft: the text stays exactly where it is, the marks come off.
///
/// One gesture for the whole summary. Reviewing is reading it once and saying yes; approving four
/// sections separately turns a two-second job into four.
pub fn confirm(paths: &Paths, meeting: &MeetingId) -> Result<Vec<String>> {
    let (path, mut doc) = read_note(paths, meeting)?;

    let approved = summo_vault::pending::approve_all(&mut doc);
    if approved.is_empty() {
        return Err(Error::Other(format!(
            "meeting {meeting} has nothing waiting to be confirmed"
        )));
    }
    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;
    clear_sidecar(paths, meeting);
    Ok(approved)
}

/// Write sections into the note, marked as the agent's and unapproved.
fn put(paths: &Paths, meeting: &MeetingId, sections: &[Section]) -> Result<()> {
    let (path, mut doc) = read_note(paths, meeting)?;
    for section in sections {
        summo_vault::pending::set_draft(&mut doc, &section.heading, section.body.trim());
    }
    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())
}

/// Produce the first draft.
pub async fn generate(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    template_id: Option<&str>,
) -> Result<Draft> {
    let (_, doc) = read_note(paths, meeting)?;
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

    put(paths, meeting, &sections)?;
    write_sidecar(
        paths,
        meeting,
        &Sidecar {
            template: template.id.clone(),
            turns: Vec::new(),
            revisions: 0,
        },
    )?;
    Ok(Draft {
        meeting: meeting.clone(),
        template: template.id.clone(),
        sections,
        turns: Vec::new(),
        revisions: 0,
    })
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
    persist(paths, &draft)?;
    Ok(draft)
}

fn persist(paths: &Paths, draft: &Draft) -> Result<()> {
    put(paths, &draft.meeting, &draft.sections)?;
    write_sidecar(
        paths,
        &draft.meeting,
        &Sidecar {
            template: draft.template.clone(),
            turns: draft.turns.clone(),
            revisions: draft.revisions,
        },
    )
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

    let (_, doc) = read_note(paths, meeting)?;
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
    persist(paths, &draft)?;
    Ok(draft)
}

/// One document, by the id the library gave it.
///
/// Through `summo_vault::open` rather than `MeetingDoc::parse`, which is the difference between
/// reading a file somebody wrote and refusing it. `open`'s own comment counted the doors that were
/// still shut — "`summo summarize`, `summo export`, `summo dub`, the daemon's summariser" — and
/// this was a fifth: every draft operation parsed the markdown itself, so a note written in any
/// other editor, with no frontmatter for a parser to find, answered 400 to the screen that had
/// just opened it.
fn read_note(paths: &Paths, meeting: &MeetingId) -> Result<(std::path::PathBuf, MeetingDoc)> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::of_vault(&vault)?;
    let entry = index
        .entries()
        .iter()
        .find(|e| &e.id == meeting)
        .ok_or_else(|| Error::Other(format!("no meeting with id {meeting}")))?;
    let path = vault.join(&entry.path);
    let doc = summo_vault::open(&vault, &path)?;
    Ok((path, doc))
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
             participants: []\ntags: []\n---\n# Họp\n\n## Ghi chú của tôi\nTôi tự viết.\n",
        )
        .unwrap();
        (dir, paths)
    }

    fn meeting() -> MeetingId {
        MeetingId::from("01A".to_string())
    }

    fn drafted(paths: &Paths) {
        put(
            paths,
            &meeting(),
            &[
                Section {
                    heading: "Tóm tắt".into(),
                    body: "Câu một. Câu hai.".into(),
                },
                Section {
                    heading: "Quyết định".into(),
                    body: "- Dùng Rust".into(),
                },
            ],
        )
        .expect("put");
    }

    fn note(paths: &Paths) -> String {
        std::fs::read_to_string(paths.meetings().join("01A.md")).unwrap()
    }

    #[test]
    fn a_meeting_with_no_draft_has_none() {
        let (_d, paths) = vault();
        assert!(load(&paths, &meeting()).expect("load").is_none());
    }

    /// The change from the first design: the text is in the note straight away, marked.
    #[test]
    fn a_draft_is_written_into_the_note_marked() {
        let (_d, paths) = vault();
        drafted(&paths);

        let body = note(&paths);
        assert!(body.contains("Câu một."), "{body}");
        assert!(body.contains("## Tóm tắt <!-- summo:draft -->"), "{body}");
        assert!(
            body.contains("Tôi tự viết."),
            "a human's section is untouched"
        );
    }

    #[test]
    fn loading_reads_the_marked_sections_back() {
        let (_d, paths) = vault();
        drafted(&paths);

        let draft = load(&paths, &meeting()).expect("load").expect("some");
        assert_eq!(draft.sections.len(), 2);
        assert_eq!(
            draft.sections[0].heading, "Tóm tắt",
            "the mark is not part of the heading"
        );
        assert_eq!(draft.sections[1].body, "- Dùng Rust");
    }

    #[test]
    fn a_humans_own_section_is_never_part_of_the_draft() {
        let (_d, paths) = vault();
        drafted(&paths);
        let draft = load(&paths, &meeting()).expect("load").expect("some");
        assert!(
            !draft
                .sections
                .iter()
                .any(|s| s.heading == "Ghi chú của tôi"),
            "{:?}",
            draft.sections
        );
    }

    #[test]
    fn confirming_keeps_the_text_and_removes_the_marks() {
        let (_d, paths) = vault();
        drafted(&paths);

        let approved = confirm(&paths, &meeting()).expect("confirm");
        assert_eq!(approved, vec!["Tóm tắt", "Quyết định"]);

        let body = note(&paths);
        assert!(body.contains("Câu một."), "the text stays: {body}");
        assert!(!body.contains("summo:draft"), "the marks go: {body}");
        assert!(load(&paths, &meeting()).expect("load").is_none());
    }

    #[test]
    fn confirming_when_there_is_nothing_waiting_is_an_error() {
        let (_d, paths) = vault();
        assert!(confirm(&paths, &meeting()).is_err());
    }

    #[test]
    fn confirming_twice_is_an_error_not_a_second_write() {
        let (_d, paths) = vault();
        drafted(&paths);
        confirm(&paths, &meeting()).expect("first");
        assert!(confirm(&paths, &meeting()).is_err());
    }

    /// Rejecting the agent must never delete something a person wrote.
    #[test]
    fn discarding_removes_only_the_agents_sections() {
        let (_d, paths) = vault();
        drafted(&paths);

        assert!(discard(&paths, &meeting()).expect("discard"));
        let body = note(&paths);
        assert!(!body.contains("Câu một."), "{body}");
        assert!(body.contains("Tôi tự viết."), "{body}");
        assert!(
            !discard(&paths, &meeting()).expect("again"),
            "nothing left to discard"
        );
    }

    #[test]
    fn redrafting_replaces_rather_than_stacking() {
        let (_d, paths) = vault();
        drafted(&paths);
        put(
            &paths,
            &meeting(),
            &[Section {
                heading: "Tóm tắt".into(),
                body: "Bản hai.".into(),
            }],
        )
        .expect("put");

        let draft = load(&paths, &meeting()).expect("load").expect("some");
        let summaries: Vec<_> = draft
            .sections
            .iter()
            .filter(|s| s.heading == "Tóm tắt")
            .collect();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].body, "Bản hai.");
    }

    #[test]
    fn the_conversation_survives_but_lives_beside_the_note() {
        let (_d, paths) = vault();
        drafted(&paths);
        write_sidecar(
            &paths,
            &meeting(),
            &Sidecar {
                template: "standard".into(),
                turns: vec![Turn {
                    role: "you".into(),
                    text: "ngắn hơn".into(),
                }],
                revisions: 1,
            },
        )
        .expect("sidecar");

        let draft = load(&paths, &meeting()).expect("load").expect("some");
        assert_eq!(draft.revisions, 1);
        assert_eq!(draft.turns.len(), 1);
        assert!(
            !note(&paths).contains("ngắn hơn"),
            "the conversation is not in the user's prose"
        );
    }

    #[test]
    fn confirming_clears_the_conversation_too() {
        let (_d, paths) = vault();
        drafted(&paths);
        write_sidecar(
            &paths,
            &meeting(),
            &Sidecar {
                template: "standard".into(),
                turns: vec![],
                revisions: 3,
            },
        )
        .expect("sidecar");

        confirm(&paths, &meeting()).expect("confirm");
        assert!(!path_for(&paths, &meeting()).exists());
    }

    #[test]
    fn a_corrupt_conversation_does_not_hide_the_draft() {
        let (_d, paths) = vault();
        drafted(&paths);
        let path = path_for(&paths, &meeting());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        // Losing the history panel is a smaller loss than losing the summary.
        let draft = load(&paths, &meeting()).expect("load").expect("some");
        assert_eq!(draft.sections.len(), 2);
        assert!(draft.turns.is_empty());
    }

    #[test]
    fn markdown_round_trips_through_sections() {
        let sections = vec![
            Section {
                heading: "A".into(),
                body: "một".into(),
            },
            Section {
                heading: "B".into(),
                body: "hai".into(),
            },
        ];
        let draft = Draft {
            meeting: meeting(),
            template: "standard".into(),
            sections: sections.clone(),
            turns: Vec::new(),
            revisions: 0,
        };
        assert_eq!(Draft::sections_from(&draft.to_markdown()), sections);
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
