//! A meeting as a Markdown document.
//!
//! The format is chosen to be readable and editable by hand first, and machine-parseable second:
//! YAML frontmatter for metadata, `##` sections for content, and one line per utterance in the
//! transcript.
//!
//! ```markdown
//! ---
//! id: 01J...
//! date: 2026-08-09T10:00:00+07:00
//! duration: 2538
//! participants: ["[[Bạn]]", "[[Ngọc]]"]
//! tags: [weekly, product]
//! ---
//! # Weekly Sync
//!
//! ## Tóm tắt
//! Chốt dùng Rust cho phần lõi. [^t=12:04]
//!
//! ## Transcript
//! **[00:12:04] Bạn** — Anh nghĩ mình nên…
//! ```
//!
//! Parsing is deliberately forgiving. These files get edited in other tools, and an unrecognised
//! section must survive a round trip rather than being silently dropped — losing a user's notes
//! because they added a heading we did not expect would be unforgivable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use summo_core::{
    Error, MeetingId, Result, SpeakerId,
    segment::{Lane, Segment},
};

/// Section heading for the transcript body. Everything after it is parsed as utterances.
const TRANSCRIPT_HEADING: &str = "Transcript";

/// Document metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: MeetingId,
    /// ISO-8601 with offset, so a meeting keeps the wall-clock time it happened at.
    pub date: String,
    /// Recording length in seconds.
    #[serde(default)]
    pub duration: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Which models produced this transcript, for reproducing or re-running it later.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// Schema version, so a future format change can migrate rather than guess.
    #[serde(default = "default_schema")]
    pub schema: u32,
}

fn default_schema() -> u32 {
    1
}

impl Frontmatter {
    #[must_use]
    pub fn new(id: MeetingId, date: impl Into<String>) -> Self {
        Self {
            id,
            date: date.into(),
            duration: 0,
            participants: Vec::new(),
            tags: Vec::new(),
            models: BTreeMap::new(),
            schema: 1,
        }
    }
}

/// One meeting document.
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingDoc {
    pub frontmatter: Frontmatter,
    pub title: String,
    /// Sections other than the transcript, in document order. Includes anything a user added.
    pub sections: Vec<Section>,
    pub transcript: Vec<Segment>,
}

/// A `##` section and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub heading: String,
    pub body: String,
}

impl MeetingDoc {
    #[must_use]
    pub fn new(frontmatter: Frontmatter, title: impl Into<String>) -> Self {
        Self {
            frontmatter,
            title: title.into(),
            sections: Vec::new(),
            transcript: Vec::new(),
        }
    }

    /// Replace or insert a section by heading.
    pub fn set_section(&mut self, heading: &str, body: impl Into<String>) {
        let body = body.into();
        if let Some(existing) = self.sections.iter_mut().find(|s| s.heading == heading) {
            existing.body = body;
        } else {
            self.sections.push(Section {
                heading: heading.to_string(),
                body,
            });
        }
    }

    #[must_use]
    pub fn section(&self, heading: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| s.heading == heading)
            .map(|s| s.body.as_str())
    }

    /// Wikilink targets referenced anywhere in the document.
    ///
    /// This is what turns a folder of transcripts into a graph: a person mentioned in a summary
    /// links to their page, and their page backlinks to every meeting they appear in.
    #[must_use]
    pub fn links(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut push_from = |text: &str| {
            for link in extract_links(text) {
                if !out.contains(&link) {
                    out.push(link);
                }
            }
        };
        for p in &self.frontmatter.participants {
            push_from(p);
        }
        for s in &self.sections {
            push_from(&s.body);
        }
        out
    }

    /// Render to Markdown.
    pub fn to_markdown(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.frontmatter)
            .map_err(|e| Error::Vault(format!("cannot serialize frontmatter: {e}")))?;

        let mut out = String::with_capacity(4096);
        out.push_str("---\n");
        out.push_str(&yaml);
        out.push_str("---\n\n");
        out.push_str(&format!("# {}\n", self.title));

        for section in &self.sections {
            out.push_str(&format!(
                "\n## {}\n{}\n",
                section.heading,
                section.body.trim_end()
            ));
        }

        if !self.transcript.is_empty() {
            out.push_str(&format!("\n## {TRANSCRIPT_HEADING}\n"));
            for segment in &self.transcript {
                out.push_str(&format!("{}\n", render_segment(segment)));
            }
        }
        Ok(out)
    }

    /// Parse a Markdown document.
    pub fn parse(markdown: &str) -> Result<Self> {
        let (yaml, body) = split_frontmatter(markdown)
            .ok_or_else(|| Error::Vault("missing YAML frontmatter".into()))?;

        let frontmatter: Frontmatter = serde_yaml::from_str(yaml)
            .map_err(|e| Error::Vault(format!("cannot parse frontmatter: {e}")))?;
        if frontmatter.schema > 1 {
            return Err(Error::Vault(format!(
                "document schema {} is newer than this build understands",
                frontmatter.schema
            )));
        }

        let mut title = String::new();
        let mut sections: Vec<Section> = Vec::new();
        let mut transcript = Vec::new();
        let mut current: Option<Section> = None;
        let mut in_transcript = false;

        for line in body.lines() {
            // Only the first `# ` is the title; a later one belongs to whatever section it is in.
            if let Some(rest) = line.strip_prefix("# ")
                && title.is_empty()
            {
                title = rest.trim().to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("## ") {
                if let Some(section) = current.take() {
                    sections.push(trim_section(section));
                }
                let heading = rest.trim().to_string();
                in_transcript = heading == TRANSCRIPT_HEADING;
                if !in_transcript {
                    current = Some(Section {
                        heading,
                        body: String::new(),
                    });
                }
                continue;
            }

            if in_transcript {
                if let Some(segment) = parse_segment(line, transcript.len() as u64) {
                    transcript.push(segment);
                }
            } else if let Some(section) = current.as_mut() {
                section.body.push_str(line);
                section.body.push('\n');
            }
        }
        if let Some(section) = current.take() {
            sections.push(trim_section(section));
        }

        Ok(Self {
            frontmatter,
            title,
            sections,
            transcript,
        })
    }
}

fn trim_section(mut section: Section) -> Section {
    section.body = section.body.trim().to_string();
    section
}

/// Split `---\n…\n---\n` from the rest.
fn split_frontmatter(markdown: &str) -> Option<(&str, &str)> {
    let rest = markdown.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n');
    Some((yaml, body))
}

/// `**[00:12:04] Bạn** — text`
fn render_segment(segment: &Segment) -> String {
    let speaker = segment.speaker.as_ref().map_or("?", SpeakerId::as_str);
    format!(
        "**[{}] {}** — {}",
        format_timestamp(segment.t0),
        speaker,
        segment.text.trim()
    )
}

fn parse_segment(line: &str, seq: u64) -> Option<Segment> {
    let line = line.trim();
    let rest = line.strip_prefix("**[")?;
    let (timestamp, rest) = rest.split_once("] ")?;
    let (speaker, text) = rest.split_once("** — ")?;

    let t0 = parse_timestamp(timestamp)?;
    let speaker = SpeakerId::from(speaker.trim().to_string());
    // A file may be hand-edited, and the mic/system distinction is not recorded per line; the lane
    // is recovered from the speaker rather than guessed.
    let lane = if speaker.is_me() {
        Lane::Mic
    } else {
        Lane::System
    };

    let mut segment = Segment::new(seq, lane, text.trim(), t0, t0);
    segment.speaker = Some(speaker);
    segment.source = summo_core::segment::SegmentSource::Final;
    Some(segment)
}

/// Seconds to `HH:MM:SS`.
#[must_use]
pub fn format_timestamp(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

/// `HH:MM:SS` or `MM:SS` to seconds.
#[must_use]
pub fn parse_timestamp(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let nums: Option<Vec<u64>> = parts.iter().map(|p| p.trim().parse().ok()).collect();
    let nums = nums?;
    match nums.as_slice() {
        [h, m, s] => Some((h * 3600 + m * 60 + s) as f64),
        [m, s] => Some((m * 60 + s) as f64),
        _ => None,
    }
}

/// Extract `[[target]]` link targets, ignoring any `|alias` part.
#[must_use]
pub fn extract_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let target = after[..end].split('|').next().unwrap_or("").trim();
        if !target.is_empty() {
            out.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::segment::SegmentSource;

    fn sample() -> MeetingDoc {
        let mut fm = Frontmatter::new(
            MeetingId::from("01J8XYZ".to_string()),
            "2026-08-09T10:00:00+07:00",
        );
        fm.duration = 2538;
        fm.participants = vec!["[[Bạn]]".into(), "[[Ngọc]]".into()];
        fm.tags = vec!["weekly".into(), "product".into()];
        fm.models.insert("live".into(), "gipformer-65m".into());

        let mut doc = MeetingDoc::new(fm, "Weekly Sync");
        doc.set_section("Tóm tắt", "Chốt dùng Rust cho phần lõi. [^t=12:04]");
        doc.set_section("Action items", "- [ ] @Ngọc chốt spec API — hạn 12/08");

        let mut a = Segment::new(0, Lane::Mic, "Anh nghĩ mình nên dùng Rust", 724.0, 730.0);
        a.speaker = Some(SpeakerId::me());
        a.source = SegmentSource::Final;
        let mut b = Segment::new(1, Lane::System, "Vấn đề là API còn thiếu", 749.0, 755.0);
        b.speaker = Some(SpeakerId::from("Ngọc".to_string()));
        b.source = SegmentSource::Final;
        doc.transcript = vec![a, b];
        doc
    }

    #[test]
    fn a_document_survives_a_round_trip() {
        let doc = sample();
        let markdown = doc.to_markdown().unwrap();
        let back = MeetingDoc::parse(&markdown).unwrap();

        assert_eq!(back.title, doc.title);
        assert_eq!(back.frontmatter, doc.frontmatter);
        assert_eq!(back.sections, doc.sections);
        assert_eq!(back.transcript.len(), doc.transcript.len());
        assert_eq!(back.transcript[0].text, "Anh nghĩ mình nên dùng Rust");
        assert_eq!(
            back.transcript[1].speaker.as_ref().unwrap().as_str(),
            "Ngọc"
        );
    }

    #[test]
    fn the_rendered_file_is_readable_markdown() {
        let markdown = sample().to_markdown().unwrap();
        assert!(markdown.starts_with("---\n"));
        assert!(markdown.contains("\n# Weekly Sync\n"));
        assert!(markdown.contains("\n## Tóm tắt\n"));
        assert!(markdown.contains("**[00:12:04] me** — Anh nghĩ mình nên dùng Rust"));
    }

    #[test]
    fn sections_added_by_hand_are_not_dropped() {
        // Somebody adds their own notes in Obsidian; rewriting the file must preserve them.
        let mut markdown = sample().to_markdown().unwrap();
        markdown.push_str("\n## Ghi chú riêng\nĐừng quên hỏi về ngân sách.\n");

        let doc = MeetingDoc::parse(&markdown).unwrap();
        assert_eq!(
            doc.section("Ghi chú riêng"),
            Some("Đừng quên hỏi về ngân sách.")
        );

        let rewritten = doc.to_markdown().unwrap();
        assert!(
            rewritten.contains("Đừng quên hỏi về ngân sách."),
            "a hand-written section was lost on rewrite"
        );
    }

    #[test]
    fn missing_frontmatter_is_an_error_not_a_guess() {
        assert!(MeetingDoc::parse("# Just a heading\n\nsome text").is_err());
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_misread() {
        let markdown = "---\nid: x\ndate: 2026-08-09\nschema: 99\n---\n# Title\n";
        let err = MeetingDoc::parse(markdown).unwrap_err();
        assert!(err.to_string().contains("newer"), "got: {err}");
    }

    #[test]
    fn timestamps_round_trip_through_both_formats() {
        assert_eq!(format_timestamp(724.0), "00:12:04");
        assert_eq!(format_timestamp(3_725.0), "01:02:05");
        assert_eq!(parse_timestamp("00:12:04"), Some(724.0));
        assert_eq!(parse_timestamp("12:04"), Some(724.0));
        assert_eq!(parse_timestamp("nonsense"), None);
    }

    #[test]
    fn negative_time_does_not_underflow() {
        assert_eq!(format_timestamp(-5.0), "00:00:00");
    }

    #[test]
    fn wikilinks_are_extracted_with_aliases_stripped() {
        assert_eq!(
            extract_links("Gặp [[Ngọc]] và [[Trần Văn A|anh A]] hôm nay"),
            vec!["Ngọc", "Trần Văn A"]
        );
        assert!(extract_links("no links here").is_empty());
        assert!(extract_links("[[unclosed").is_empty());
    }

    #[test]
    fn document_links_span_participants_and_sections() {
        let mut doc = sample();
        doc.set_section("Tóm tắt", "Theo [[Ngọc]] thì [[API]] chưa xong.");
        let links = doc.links();

        assert!(links.contains(&"Ngọc".to_string()));
        assert!(links.contains(&"API".to_string()));
        assert_eq!(
            links.iter().filter(|l| *l == "Ngọc").count(),
            1,
            "a repeated link should appear once: {links:?}"
        );
    }

    #[test]
    fn a_transcript_line_that_is_not_a_line_is_skipped() {
        // Hand-edited files contain stray blank lines and prose; those must not become segments.
        let markdown = "---\nid: x\ndate: 2026-08-09\n---\n# T\n\n## Transcript\n\
             \n**[00:00:01] me** — thật\nnot a segment line\n\n";
        let doc = MeetingDoc::parse(markdown).unwrap();
        assert_eq!(doc.transcript.len(), 1);
        assert_eq!(doc.transcript[0].text, "thật");
    }

    #[test]
    fn setting_a_section_twice_replaces_rather_than_duplicates() {
        let mut doc = sample();
        doc.set_section("Tóm tắt", "bản mới");
        assert_eq!(doc.section("Tóm tắt"), Some("bản mới"));
        assert_eq!(
            doc.sections
                .iter()
                .filter(|s| s.heading == "Tóm tắt")
                .count(),
            1
        );
    }

    #[test]
    fn the_local_speaker_maps_back_to_the_mic_lane() {
        let doc = MeetingDoc::parse(&sample().to_markdown().unwrap()).unwrap();
        assert_eq!(doc.transcript[0].lane, Lane::Mic);
        assert_eq!(doc.transcript[1].lane, Lane::System);
    }
}
