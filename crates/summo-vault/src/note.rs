//! Notes that were never recorded.
//!
//! Summo is a note app whose notes happen to have audio attached to most of them. A meeting note and
//! a typed note are the *same document* — same frontmatter, same sections, same `- [ ] @ngoc …`
//! task lines, same draft markers — and the only difference is whether anything was said out loud.
//!
//! So there is no `NoteDoc`. A note is a [`MeetingDoc`] with an empty transcript, which is what
//! makes every feature already built work on it for free: search finds it, the task boards collect
//! from it, the agent can read and revise it, the MCP server serves it. Introducing a second
//! document type would have meant a second parser, a second index and a second set of bugs, and
//! would have made "add this to my note" mean something different depending on which kind it was.
//!
//! They live under `vault/notes/` rather than `vault/meetings/` for one reason: the library screen
//! is organised by *when a meeting happened*, and a note somebody typed on a Tuesday has no such
//! day. Keeping them apart is a filing decision, not a structural one — the parser does not care.

use std::path::{Path, PathBuf};

use summo_core::{Error, MeetingId, Result, paths::Paths};

use crate::meeting::{Frontmatter, MeetingDoc};
use crate::slug::meeting_stem;

/// Where notes live.
#[must_use]
pub fn dir(paths: &Paths) -> PathBuf {
    paths.notes()
}

/// Whether a document is a note rather than a recording.
///
/// The transcript is the whole test. A meeting whose audio was pruned still has its transcript, and
/// a note that quotes somebody has no segments — so this asks the question that actually
/// distinguishes them rather than inspecting the path, which would make moving a file change what
/// it is.
#[must_use]
pub fn is_note(doc: &MeetingDoc) -> bool {
    doc.transcript.is_empty()
}

/// Create a note.
///
/// `body` is Markdown and is stored as written, under no heading: a note the user typed is theirs,
/// and wrapping it in a section Summo invented would show up the first time they opened the file in
/// any other editor.
pub fn create(paths: &Paths, title: &str, day: &str, body: &str) -> Result<(MeetingId, PathBuf)> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::msg("note.no_title", "ghi chú cần có tiêu đề"));
    }

    let id = MeetingId::new();
    let mut doc = MeetingDoc::new(Frontmatter::new(id.clone(), day), title);
    if !body.trim().is_empty() {
        doc.body = body.trim().to_string();
    }

    let dir = dir(paths);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

    let path = unique(&dir, &meeting_stem(day, title));
    crate::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;
    Ok((id, path))
}

/// Every note, newest first.
///
/// Reuses [`crate::index::MeetingIndex`] because a note is the same document — a second scanner
/// would eventually disagree with the first about which file is which.
pub fn list(paths: &Paths) -> Result<Vec<crate::index::MeetingEntry>> {
    let dir = dir(paths);
    if !dir.exists() {
        // No notes yet is not an error, and creating the directory to find that out would leave an
        // empty folder in a vault the user browses.
        return Ok(Vec::new());
    }
    let index = crate::index::MeetingIndex::scan(&dir)?;
    let mut entries = index.entries().to_vec();
    entries.sort_by(|a, b| b.ordering_key().cmp(&a.ordering_key()));
    Ok(entries)
}

/// Find one note's file by id.
pub fn find(paths: &Paths, id: &MeetingId) -> Result<PathBuf> {
    let dir = dir(paths);
    let index = crate::index::MeetingIndex::scan(&dir)?;
    index
        .entries()
        .iter()
        .find(|entry| &entry.id == id)
        .map(|entry| dir.join(&entry.path))
        .ok_or_else(|| Error::msg("note.not_found", format!("không có ghi chú nào tên {id}")))
}

/// Read a note.
pub fn read(paths: &Paths, id: &MeetingId) -> Result<MeetingDoc> {
    let path = find(paths, id)?;
    crate::index::open(&paths.vault(), &path)
}

/// Replace a note's body, and optionally its title.
///
/// The *file name* is never changed, whatever the title becomes. Those are two different things and
/// conflating them costs the user their links: renaming on every save means a note called "Ý tưởng"
/// becomes a different file the moment they finish the sentence. The heading inside the document is
/// what a person reads, and that does change.
///
/// A blank title is ignored rather than applied — an editor whose first line is momentarily empty
/// is an editor mid-edit, not a request to un-name the note.
pub fn set_body(paths: &Paths, id: &MeetingId, body: &str, title: Option<&str>) -> Result<PathBuf> {
    let path = find(paths, id)?;
    let mut doc = crate::index::open(&paths.vault(), &path)?;

    if let Some(title) = title
        && !title.trim().is_empty()
    {
        doc.title = title.trim().to_string();
    }
    // Headings in the text are headings in the document, not four literal characters at the start
    // of a line. Assigning the whole editor buffer to `body` and leaving `sections` as they were —
    // which is what this did — wrote the file with the new text *and* the old sections still under
    // it, so a note with any `##` in it grew a duplicate of itself on every save. It went unnoticed
    // because the editor only ever displayed `body`, so the duplicate was invisible in the one
    // place somebody would have seen it.
    let (lead, sections) = split_sections(body);
    doc.body = lead;
    doc.sections = sections;
    crate::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;
    Ok(path)
}

/// Split editor text into the free part and the `##` sections under it.
///
/// The transcript heading is *not* special-cased here: notes have no transcript, and a note whose
/// user typed a heading with that name has typed a heading.
#[must_use]
pub fn split_sections(text: &str) -> (String, Vec<crate::meeting::Section>) {
    let mut lead = String::new();
    let mut sections: Vec<crate::meeting::Section> = Vec::new();

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.push(crate::meeting::Section {
                heading: heading.trim().to_string(),
                body: String::new(),
            });
            continue;
        }
        match sections.last_mut() {
            Some(section) => {
                section.body.push_str(line);
                section.body.push('\n');
            }
            None => {
                lead.push_str(line);
                lead.push('\n');
            }
        }
    }

    for section in &mut sections {
        section.body = section.body.trim_end().to_string();
    }
    (lead.trim().to_string(), sections)
}

/// The whole note as one piece of text, the way the editor shows it: everything under the title.
///
/// The document keeps body and sections apart, which is right for a *meeting* — the summary's
/// sections are addressable, and the draft machinery rewrites one of them at a time. A note is one
/// document a person is typing in, and handing the editor only `body` is what made a note written
/// in Obsidian open with most of itself missing.
#[must_use]
pub fn as_text(doc: &crate::meeting::MeetingDoc) -> String {
    let mut text = doc.body.trim().to_string();
    for section in &doc.sections {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!(
            "## {}\n{}",
            section.heading,
            section.body.trim_end()
        ));
    }
    text
}

/// Delete a note. `false` when there was nothing there.
pub fn remove(paths: &Paths, id: &MeetingId) -> Result<bool> {
    let Ok(path) = find(paths, id) else {
        return Ok(false);
    };
    std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
    Ok(true)
}

/// A path that does not collide with a note already there.
///
/// Two notes a day can easily share a title — "ý tưởng" every morning — and silently overwriting
/// yesterday's would be data loss disguised as a naming convention.
fn unique(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.md"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..1000 {
        let candidate = dir.join(format!("{stem}-{suffix}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.md", MeetingId::new()))
}

#[cfg(test)]
mod tests {
    /// The bug this pair of functions exists for: a note with headings used to open with everything
    /// below the first `##` missing, and saving it back wrote the file with the visible part *and*
    /// the invisible sections still underneath — a duplicate of the note, growing on every save.
    #[test]
    fn a_note_with_headings_survives_a_round_trip_through_the_editor() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let seed = "## Quyết định\nChốt giá 12 triệu.\n\n## Bối cảnh\nACME muốn thử hai tuần.";
        let (id, path) = create(&paths, "Chốt giá", "2026-08-14", seed).unwrap();

        let shown = as_text(&read(&paths, &id).unwrap());
        assert!(
            shown.contains("## Quyết định"),
            "the editor sees the headings: {shown}"
        );
        assert!(shown.contains("## Bối cảnh"));
        assert!(shown.contains("ACME muốn thử hai tuần."));

        set_body(
            &paths,
            &id,
            &format!("{shown}\n\nNgọc chốt."),
            Some("Chốt giá"),
        )
        .unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            on_disk.matches("## Quyết định").count(),
            1,
            "saving must not duplicate the sections:\n{on_disk}"
        );
        assert!(on_disk.contains("Ngọc chốt."));
        let again = as_text(&read(&paths, &id).unwrap());
        assert!(again.contains("## Bối cảnh") && again.contains("Ngọc chốt."));
    }

    #[test]
    fn text_with_no_headings_is_left_exactly_as_it_is() {
        let (lead, sections) = split_sections("một dòng\nhai dòng");
        assert_eq!(lead, "một dòng\nhai dòng");
        assert!(sections.is_empty());
    }

    use super::*;

    fn vault() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        (tmp, paths)
    }

    #[test]
    fn a_note_round_trips_through_the_same_parser_a_meeting_uses() {
        let (_tmp, paths) = vault();
        let (id, _) = create(&paths, "Ý tưởng", "2026-08-10", "Ghi nhanh vài dòng.").unwrap();

        let doc = read(&paths, &id).unwrap();
        assert_eq!(doc.title, "Ý tưởng");
        assert!(doc.body.contains("Ghi nhanh"));
        assert!(doc.transcript.is_empty());
    }

    /// The whole design in one assertion: a note is a meeting document with nothing said out loud,
    /// so everything already built for meetings works on it.
    #[test]
    fn a_note_is_told_from_a_meeting_by_its_transcript_not_its_folder() {
        use summo_core::segment::{Lane, Segment};

        let (_tmp, paths) = vault();
        let (id, _) = create(&paths, "Ghi chú", "2026-08-10", "text").unwrap();
        assert!(is_note(&read(&paths, &id).unwrap()));

        let mut recorded = MeetingDoc::new(Frontmatter::new(MeetingId::new(), "2026-08-10"), "Họp");
        recorded
            .transcript
            .push(Segment::new(1, Lane::System, "xin chào", 0.0, 1.0));
        assert!(!is_note(&recorded));
    }

    /// Task lines are parsed from the document, not from the folder, so a task typed into a note
    /// has to reach the board.
    #[test]
    fn a_task_written_in_a_note_is_found_by_the_task_parser() {
        let (_tmp, paths) = vault();
        let (id, _) = create(
            &paths,
            "Việc tuần này",
            "2026-08-10",
            "## Việc cần làm\n- [ ] @ngoc Chốt spec API <!-- id:T9 -->",
        )
        .unwrap();

        let path = find(&paths, &id).unwrap();
        let markdown = std::fs::read_to_string(&path).unwrap();
        let tasks = crate::tasks::parse(&markdown, "notes/x.md");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].owner.as_deref(), Some("ngoc"));
    }

    #[test]
    fn a_note_without_a_title_is_refused() {
        let (_tmp, paths) = vault();
        assert!(create(&paths, "   ", "2026-08-10", "x").is_err());
    }

    #[test]
    fn an_empty_body_is_allowed_because_a_note_often_starts_empty() {
        let (_tmp, paths) = vault();
        let (id, _) = create(&paths, "Trống", "2026-08-10", "").unwrap();
        assert_eq!(read(&paths, &id).unwrap().title, "Trống");
    }

    /// Two notes a day can easily share a title, and overwriting is data loss dressed as a naming
    /// convention.
    #[test]
    fn two_notes_with_the_same_title_do_not_overwrite_each_other() {
        let (_tmp, paths) = vault();
        let (first, a) = create(&paths, "Ý tưởng", "2026-08-10", "một").unwrap();
        let (second, b) = create(&paths, "Ý tưởng", "2026-08-10", "hai").unwrap();

        assert_ne!(a, b);
        assert_eq!(read(&paths, &first).unwrap().body.trim(), "một");
        assert_eq!(read(&paths, &second).unwrap().body.trim(), "hai");
    }

    #[test]
    fn notes_are_listed_newest_first() {
        let (_tmp, paths) = vault();
        create(&paths, "Cũ", "2026-08-01", "").unwrap();
        create(&paths, "Mới", "2026-08-10", "").unwrap();

        let titles: Vec<_> = list(&paths).unwrap().into_iter().map(|e| e.title).collect();
        assert_eq!(titles, ["Mới", "Cũ"]);
    }

    /// Creating the directory to discover it is empty would leave a folder in a vault the user
    /// browses in Finder.
    #[test]
    fn a_vault_with_no_notes_lists_nothing_and_creates_nothing() {
        let (_tmp, paths) = vault();
        assert!(list(&paths).unwrap().is_empty());
        assert!(!dir(&paths).exists());
    }

    /// The user has links and a folder full of names; renaming a file because they fixed a typo in
    /// the text would break both.
    #[test]
    fn editing_the_body_does_not_rename_the_file() {
        let (_tmp, paths) = vault();
        let (id, path) = create(&paths, "Ý tưởng", "2026-08-10", "một").unwrap();

        let after = set_body(&paths, &id, "hai", None).unwrap();
        assert_eq!(after, path);
        assert_eq!(read(&paths, &id).unwrap().body.trim(), "hai");
        assert_eq!(read(&paths, &id).unwrap().title, "Ý tưởng");
    }

    #[test]
    fn editing_a_note_that_is_not_there_says_so() {
        let (_tmp, paths) = vault();
        let err = set_body(&paths, &MeetingId::new(), "x", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("không có ghi chú"), "{err}");
    }

    /// The user types over the first line and expects the note to be called that. Dropping the edit
    /// because the file is named after the old title would be losing their keystrokes to a filing
    /// decision they never made.
    #[test]
    fn retitling_changes_the_heading_and_leaves_the_file_where_it_is() {
        let (_tmp, paths) = vault();
        let (id, path) = create(&paths, "Cũ", "2026-08-10", "nội dung").unwrap();

        let after = set_body(&paths, &id, "nội dung", Some("Mới")).unwrap();
        assert_eq!(after, path, "the file does not move");
        assert_eq!(read(&paths, &id).unwrap().title, "Mới");
    }

    /// An editor whose first line is momentarily empty is an editor mid-edit, not a request to
    /// un-name the note.
    #[test]
    fn a_blank_title_is_ignored_rather_than_applied() {
        let (_tmp, paths) = vault();
        let (id, _) = create(&paths, "Giữ tên", "2026-08-10", "x").unwrap();

        set_body(&paths, &id, "x", Some("   ")).unwrap();
        assert_eq!(read(&paths, &id).unwrap().title, "Giữ tên");
    }

    #[test]
    fn removing_a_note_that_is_not_there_is_not_an_error() {
        let (_tmp, paths) = vault();
        assert!(!remove(&paths, &MeetingId::new()).unwrap());

        let (id, _) = create(&paths, "Xoá tôi", "2026-08-10", "").unwrap();
        assert!(remove(&paths, &id).unwrap());
        assert!(list(&paths).unwrap().is_empty());
    }

    /// Notes and meetings are filed apart on purpose — the library is organised by when a meeting
    /// happened, and a typed note has no such day — but nothing structural depends on it.
    #[test]
    fn notes_do_not_appear_among_the_meetings() {
        let (_tmp, paths) = vault();
        std::fs::create_dir_all(paths.meetings()).unwrap();
        create(&paths, "Ghi chú", "2026-08-10", "").unwrap();

        let index = crate::index::MeetingIndex::scan(paths.meetings()).unwrap();
        assert!(index.entries().is_empty());
    }
}
