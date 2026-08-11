//! Sections the agent wrote that nobody has approved yet.
//!
//! The agent's summary goes straight into the note, where the user reads it in context — but marked,
//! so it is obvious which paragraphs a person wrote and which a model did. Confirming removes the
//! mark; that is the whole gesture.
//!
//! ```markdown
//! ## Tóm tắt <!-- summo:draft -->
//! Chốt ngân sách quý 4.
//! ```
//!
//! The marker is an HTML comment for one reason: it disappears when the Markdown is rendered. The
//! note opens in Obsidian looking finished, greps like any other file, and syncs without carrying a
//! private sidecar — while Summo can still tell the two apart and tint the unapproved parts.
//!
//! The alternative was a draft in a separate file. It is worse in a way that only shows up in use:
//! a note whose summary is hidden somewhere else *looks like a note with no summary*, so the user
//! opens it, sees nothing, and wonders whether the app worked.

/// What marks a section as written-but-unapproved.
///
/// Deliberately on the heading line rather than in frontmatter: a section that is moved, renamed or
/// deleted by hand takes its own status with it, instead of leaving a stale entry in a list at the
/// top of the file.
pub const MARKER: &str = "<!-- summo:draft -->";

/// Whether a heading carries the marker.
#[must_use]
pub fn is_draft(heading: &str) -> bool {
    heading.trim_end().ends_with(MARKER)
}

/// The heading without its marker, which is what the user reads and what `set_section` matches on.
#[must_use]
pub fn strip(heading: &str) -> &str {
    match heading.trim_end().strip_suffix(MARKER) {
        Some(rest) => rest.trim_end(),
        None => heading.trim_end(),
    }
}

/// The heading with the marker, ready to write.
#[must_use]
pub fn mark(heading: &str) -> String {
    let clean = strip(heading);
    format!("{clean} {MARKER}")
}

/// Every unapproved heading in a document, in order.
#[must_use]
pub fn in_document(doc: &crate::meeting::MeetingDoc) -> Vec<String> {
    doc.sections
        .iter()
        .filter(|s| is_draft(&s.heading))
        .map(|s| strip(&s.heading).to_string())
        .collect()
}

/// Approve one section: keep the text, drop the mark.
///
/// Returns whether anything changed, so a caller can tell "approved" from "was already approved"
/// rather than reporting success either way.
pub fn approve(doc: &mut crate::meeting::MeetingDoc, heading: &str) -> bool {
    let clean = strip(heading);
    for section in &mut doc.sections {
        if is_draft(&section.heading) && strip(&section.heading) == clean {
            section.heading = clean.to_string();
            return true;
        }
    }
    false
}

/// Approve everything in one go, which is the usual case: read the summary once, accept it.
pub fn approve_all(doc: &mut crate::meeting::MeetingDoc) -> Vec<String> {
    let mut approved = Vec::new();
    for section in &mut doc.sections {
        if is_draft(&section.heading) {
            let clean = strip(&section.heading).to_string();
            section.heading = clean.clone();
            approved.push(clean);
        }
    }
    approved
}

/// Remove an unapproved section entirely.
///
/// Only touches marked sections: rejecting the agent's draft must never delete something a human
/// wrote under the same heading.
pub fn reject(doc: &mut crate::meeting::MeetingDoc, heading: &str) -> bool {
    let clean = strip(heading);
    let before = doc.sections.len();
    doc.sections
        .retain(|s| !(is_draft(&s.heading) && strip(&s.heading) == clean));
    doc.sections.len() != before
}

/// Write a section and mark it as the agent's, unapproved.
pub fn set_draft(doc: &mut crate::meeting::MeetingDoc, heading: &str, body: impl Into<String>) {
    let clean = strip(heading);
    let body = body.into();

    // Replace whichever form is already there, so re-drafting does not leave two copies.
    for section in &mut doc.sections {
        if strip(&section.heading) == clean {
            section.heading = mark(clean);
            section.body = body;
            return;
        }
    }
    doc.sections.push(crate::meeting::Section {
        heading: mark(clean),
        body,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::{Frontmatter, MeetingDoc};
    use summo_core::MeetingId;

    fn doc() -> MeetingDoc {
        MeetingDoc::new(
            Frontmatter::new(
                MeetingId::from("01A".to_string()),
                "2026-08-10T09:00:00+07:00",
            ),
            "Họp",
        )
    }

    #[test]
    fn a_marked_heading_is_recognised_and_cleaned() {
        assert!(is_draft("Tóm tắt <!-- summo:draft -->"));
        assert!(!is_draft("Tóm tắt"));
        assert_eq!(strip("Tóm tắt <!-- summo:draft -->"), "Tóm tắt");
        assert_eq!(strip("Tóm tắt"), "Tóm tắt");
    }

    #[test]
    fn marking_is_idempotent() {
        let once = mark("Tóm tắt");
        assert_eq!(mark(&once), once, "marking twice must not stack markers");
    }

    #[test]
    fn a_draft_section_is_written_marked() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Nội dung");
        assert_eq!(in_document(&d), vec!["Tóm tắt"]);
        assert!(
            d.to_markdown()
                .unwrap()
                .contains("## Tóm tắt <!-- summo:draft -->")
        );
    }

    #[test]
    fn redrafting_replaces_rather_than_duplicating() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Bản một");
        set_draft(&mut d, "Tóm tắt", "Bản hai");
        assert_eq!(d.sections.len(), 1);
        assert_eq!(d.sections[0].body, "Bản hai");
    }

    /// Re-drafting over an approved section marks it again — it is new text nobody has read.
    #[test]
    fn redrafting_an_approved_section_marks_it_again() {
        let mut d = doc();
        d.set_section("Tóm tắt", "Người viết");
        set_draft(&mut d, "Tóm tắt", "Agent viết lại");
        assert_eq!(in_document(&d), vec!["Tóm tắt"]);
        assert_eq!(d.sections.len(), 1);
    }

    #[test]
    fn approving_keeps_the_text_and_drops_the_mark() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Nội dung");
        assert!(approve(&mut d, "Tóm tắt"));

        assert!(in_document(&d).is_empty());
        assert_eq!(d.section("Tóm tắt"), Some("Nội dung"));
        assert!(!d.to_markdown().unwrap().contains(MARKER));
    }

    #[test]
    fn approving_twice_reports_that_nothing_changed() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Nội dung");
        assert!(approve(&mut d, "Tóm tắt"));
        assert!(!approve(&mut d, "Tóm tắt"));
    }

    #[test]
    fn approving_everything_is_one_gesture() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "A");
        set_draft(&mut d, "Quyết định", "B");
        d.set_section("Ghi chú của tôi", "C");

        let approved = approve_all(&mut d);
        assert_eq!(approved, vec!["Tóm tắt", "Quyết định"]);
        assert!(in_document(&d).is_empty());
        assert_eq!(
            d.section("Ghi chú của tôi"),
            Some("C"),
            "a human's section is untouched"
        );
    }

    /// The property that matters: rejecting the agent must not delete a person's writing.
    #[test]
    fn rejecting_only_removes_the_agents_section() {
        let mut d = doc();
        d.set_section("Tóm tắt", "Tôi tự viết");
        assert!(
            !reject(&mut d, "Tóm tắt"),
            "nothing of the agent's to reject"
        );
        assert_eq!(d.section("Tóm tắt"), Some("Tôi tự viết"));

        set_draft(&mut d, "Quyết định", "Agent viết");
        assert!(reject(&mut d, "Quyết định"));
        assert!(d.section("Quyết định").is_none());
        assert_eq!(d.section("Tóm tắt"), Some("Tôi tự viết"), "still there");
    }

    #[test]
    fn a_marker_survives_a_round_trip_through_markdown() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Nội dung");
        d.set_section("Của tôi", "Người viết");

        let markdown = d.to_markdown().expect("render");
        let back = MeetingDoc::parse(&markdown).expect("parse");

        assert_eq!(in_document(&back), vec!["Tóm tắt"]);
        assert_eq!(back.section("Của tôi"), Some("Người viết"));
    }

    /// Rendered Markdown hides the comment, so the note reads as finished outside Summo.
    #[test]
    fn the_marker_is_invisible_when_rendered() {
        let mut d = doc();
        set_draft(&mut d, "Tóm tắt", "Nội dung");
        let markdown = d.to_markdown().expect("render");
        // An HTML comment: present in the source, absent from the reader's screen.
        assert!(markdown.contains("<!--") && markdown.contains("-->"));
        assert!(markdown.contains("Tóm tắt"));
    }
}
