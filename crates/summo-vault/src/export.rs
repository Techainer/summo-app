//! Getting a transcript out of Summo.
//!
//! Export exists because a local-first app has no right to be the only way to read your own data.
//! The vault is already Markdown, so export is less about escape and more about the formats other
//! tools expect: subtitle files for video editors, plain text for pasting, JSON for scripting.
//!
//! The subtitle formats are where the fiddly details live. SRT counts from 1 and separates
//! milliseconds with a comma; WebVTT counts from 0, uses a period, and needs a `WEBVTT` header.
//! Getting either wrong produces a file that loads with no subtitles and no error, which is why
//! both are pinned by test against hand-written expected output.

use serde::{Deserialize, Serialize};
use summo_core::segment::Segment;

use crate::meeting::MeetingDoc;

/// Formats a transcript can be written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// The vault's own format, unchanged.
    Markdown,
    /// Speaker and text, no timestamps. What people paste into an email.
    Text,
    /// SubRip subtitles.
    Srt,
    /// WebVTT subtitles.
    Vtt,
    /// Everything, for scripting.
    Json,
    /// Comma-separated rows, for a spreadsheet.
    Csv,
}

impl Format {
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }

    /// Parse a format from a name or an extension.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().trim_start_matches('.').to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::Text),
            "srt" => Some(Self::Srt),
            "vtt" | "webvtt" => Some(Self::Vtt),
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }

    /// Every format, for a UI to list.
    #[must_use]
    pub fn all() -> [Self; 6] {
        [
            Self::Markdown,
            Self::Text,
            Self::Srt,
            Self::Vtt,
            Self::Json,
            Self::Csv,
        ]
    }
}

/// What to include.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub timestamps: bool,
    pub speakers: bool,
    /// Merge consecutive utterances from one speaker into a paragraph.
    ///
    /// Recognition splits on breath, not on thought, so a transcript is full of two-second
    /// fragments. For reading, joining them is almost always what someone wants; for subtitles it
    /// would produce cues far too long to read.
    pub merge_by_speaker: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timestamps: true,
            speakers: true,
            merge_by_speaker: false,
        }
    }
}

impl Options {
    /// Sensible defaults for reading rather than for machines.
    #[must_use]
    pub fn readable() -> Self {
        Self {
            timestamps: false,
            speakers: true,
            merge_by_speaker: true,
        }
    }
}

/// Render a meeting in the requested format.
pub fn export(doc: &MeetingDoc, format: Format, options: Options) -> summo_core::Result<String> {
    Ok(match format {
        Format::Markdown => doc.to_markdown()?,
        Format::Text => to_text(doc, options),
        Format::Srt => to_srt(&doc.transcript, options),
        Format::Vtt => to_vtt(&doc.transcript, options),
        Format::Json => serde_json::to_string_pretty(&JsonExport::from(doc))?,
        Format::Csv => to_csv(&doc.transcript),
    })
}

/// Group consecutive utterances by speaker.
fn grouped(segments: &[Segment], merge: bool) -> Vec<Vec<&Segment>> {
    if !merge {
        return segments.iter().map(|s| vec![s]).collect();
    }
    let mut groups: Vec<Vec<&Segment>> = Vec::new();
    for segment in segments {
        match groups.last_mut() {
            Some(group) if group.last().is_some_and(|p| p.speaker == segment.speaker) => {
                group.push(segment);
            }
            _ => groups.push(vec![segment]),
        }
    }
    groups
}

fn speaker_of(segment: &Segment) -> &str {
    segment
        .speaker
        .as_ref()
        .map_or("?", summo_core::SpeakerId::as_str)
}

fn to_text(doc: &MeetingDoc, options: Options) -> String {
    let mut out = String::new();
    if !doc.title.is_empty() {
        out.push_str(&format!("{}\n\n", doc.title));
    }

    for group in grouped(&doc.transcript, options.merge_by_speaker) {
        let Some(first) = group.first() else { continue };
        let text = group
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ");

        let mut line = String::new();
        if options.timestamps {
            line.push_str(&format!("[{}] ", crate::meeting::format_timestamp(first.t0)));
        }
        if options.speakers {
            line.push_str(&format!("{}: ", speaker_of(first)));
        }
        line.push_str(&text);
        out.push_str(line.trim_end());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

/// `HH:MM:SS,mmm` for SRT, `HH:MM:SS.mmm` for WebVTT.
fn subtitle_time(seconds: f64, separator: char) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total = total_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        total / 3600,
        (total % 3600) / 60,
        total % 60,
        ms
    )
}

/// Minimum cue length. A one-word utterance would otherwise flash by unreadably.
const MIN_CUE_S: f64 = 1.0;

fn cue_end(segment: &Segment) -> f64 {
    segment.t1.max(segment.t0 + MIN_CUE_S)
}

fn to_srt(segments: &[Segment], options: Options) -> String {
    let mut out = String::new();
    // SRT cue numbering starts at 1; players reject a file starting at 0.
    for (index, segment) in segments.iter().filter(|s| !s.is_empty()).enumerate() {
        out.push_str(&format!("{}\n", index + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            subtitle_time(segment.t0, ','),
            subtitle_time(cue_end(segment), ',')
        ));
        if options.speakers {
            out.push_str(&format!("{}: ", speaker_of(segment)));
        }
        out.push_str(segment.text.trim());
        out.push_str("\n\n");
    }
    out
}

fn to_vtt(segments: &[Segment], options: Options) -> String {
    // The header is mandatory; without it a player loads the file and shows nothing.
    let mut out = String::from("WEBVTT\n\n");
    for segment in segments.iter().filter(|s| !s.is_empty()) {
        out.push_str(&format!(
            "{} --> {}\n",
            subtitle_time(segment.t0, '.'),
            subtitle_time(cue_end(segment), '.')
        ));
        if options.speakers {
            // WebVTT has a voice span for exactly this.
            out.push_str(&format!("<v {}>", speaker_of(segment)));
        }
        out.push_str(segment.text.trim());
        out.push_str("\n\n");
    }
    out
}

fn to_csv(segments: &[Segment]) -> String {
    let mut out = String::from("start_s,end_s,speaker,text\n");
    for segment in segments {
        out.push_str(&format!(
            "{:.3},{:.3},{},{}\n",
            segment.t0,
            segment.t1,
            csv_field(speaker_of(segment)),
            csv_field(segment.text.trim())
        ));
    }
    out
}

/// Quote a CSV field, doubling any quote inside it.
///
/// Transcripts contain commas constantly and quotation marks often; getting this wrong shifts every
/// column after it and corrupts the file silently.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[derive(Serialize)]
struct JsonExport<'a> {
    title: &'a str,
    #[serde(flatten)]
    frontmatter: &'a crate::meeting::Frontmatter,
    segments: &'a [Segment],
}

impl<'a> From<&'a MeetingDoc> for JsonExport<'a> {
    fn from(doc: &'a MeetingDoc) -> Self {
        Self {
            title: &doc.title,
            frontmatter: &doc.frontmatter,
            segments: &doc.transcript,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::{MeetingId, SpeakerId, segment::Lane};

    fn doc() -> MeetingDoc {
        let mut d = MeetingDoc::new(
            crate::meeting::Frontmatter::new(MeetingId::from("m1".to_string()), "2026-08-09"),
            "Weekly Sync",
        );
        let mut a = Segment::new(0, Lane::Mic, "Anh nghĩ mình nên dùng Rust", 1.0, 3.5);
        a.speaker = Some(SpeakerId::me());
        let mut b = Segment::new(1, Lane::Mic, "cho phần lõi", 3.6, 4.8);
        b.speaker = Some(SpeakerId::me());
        let mut c = Segment::new(2, Lane::System, "Vấn đề là API còn thiếu", 5.0, 7.2);
        c.speaker = Some(SpeakerId::from("Ngọc".to_string()));
        d.transcript = vec![a, b, c];
        d
    }

    #[test]
    fn formats_round_trip_through_their_names_and_extensions() {
        for format in Format::all() {
            assert_eq!(Format::parse(format.extension()), Some(format));
        }
        assert_eq!(Format::parse(".SRT"), Some(Format::Srt));
        assert_eq!(Format::parse("webvtt"), Some(Format::Vtt));
        assert_eq!(Format::parse("docx"), None);
    }

    #[test]
    fn srt_numbers_cues_from_one_and_uses_a_comma() {
        // Both are load-bearing: a file starting at 0, or using a period, shows no subtitles at all.
        let srt = to_srt(&doc().transcript, Options::default());
        let first: Vec<&str> = srt.lines().take(3).collect();

        assert_eq!(first[0], "1");
        assert_eq!(first[1], "00:00:01,000 --> 00:00:03,500");
        assert_eq!(first[2], "me: Anh nghĩ mình nên dùng Rust");
        assert!(srt.contains("\n2\n"), "the second cue should be numbered 2");
    }

    #[test]
    fn vtt_carries_the_header_and_a_voice_span() {
        let vtt = to_vtt(&doc().transcript, Options::default());
        assert!(vtt.starts_with("WEBVTT\n\n"), "a missing header renders no subtitles");
        assert!(vtt.contains("00:00:01.000 --> 00:00:03.500"), "vtt uses a period");
        assert!(vtt.contains("<v me>"));
    }

    #[test]
    fn a_very_short_utterance_still_gets_a_readable_cue() {
        let mut segments = doc().transcript;
        segments[0].t1 = segments[0].t0 + 0.2;
        let srt = to_srt(&segments, Options::default());
        assert!(
            srt.contains("00:00:01,000 --> 00:00:02,000"),
            "a 200 ms cue would flash by unreadably: {srt}"
        );
    }

    #[test]
    fn empty_segments_are_not_written_as_cues() {
        let mut segments = doc().transcript;
        segments[1].text = "   ".into();
        let srt = to_srt(&segments, Options::default());
        assert_eq!(srt.matches(" --> ").count(), 2);
    }

    #[test]
    fn readable_text_merges_a_speakers_consecutive_fragments() {
        // Recognition splits on breath, not on thought, so unmerged text reads as a list of scraps.
        let text = to_text(&doc(), Options::readable());
        assert!(
            text.contains("me: Anh nghĩ mình nên dùng Rust cho phần lõi"),
            "fragments should have been joined: {text}"
        );
        assert!(!text.contains('['), "readable output should carry no timestamps");
    }

    #[test]
    fn unmerged_text_keeps_every_utterance_separate() {
        let text = to_text(&doc(), Options::default());
        assert_eq!(text.matches("me:").count(), 2);
        assert!(text.contains("[00:00:01]"));
    }

    #[test]
    fn a_change_of_speaker_always_breaks_a_group() {
        let text = to_text(&doc(), Options::readable());
        assert!(text.contains("Ngọc: Vấn đề là API còn thiếu"));
    }

    #[test]
    fn csv_quotes_fields_containing_commas_and_quotes() {
        // A transcript is full of commas; getting this wrong shifts every later column.
        let mut segments = doc().transcript;
        segments[0].text = r#"anh nói "được", rồi đi"#.into();
        let csv = to_csv(&segments);
        let line = csv.lines().nth(1).unwrap();

        assert!(line.contains(r#""anh nói ""được"", rồi đi""#), "got: {line}");
        assert_eq!(csv.lines().count(), 4, "header plus three rows");
    }

    #[test]
    fn json_export_carries_metadata_and_segments() {
        let json = export(&doc(), Format::Json, Options::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["title"], "Weekly Sync");
        assert_eq!(value["id"], "m1");
        assert_eq!(value["segments"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn speakers_can_be_left_out_entirely() {
        let options = Options {
            speakers: false,
            ..Options::default()
        };
        assert!(!to_srt(&doc().transcript, options).contains("me:"));
        assert!(!to_vtt(&doc().transcript, options).contains("<v "));
    }

    #[test]
    fn every_format_produces_something_for_a_real_meeting() {
        for format in Format::all() {
            let out = export(&doc(), format, Options::default()).unwrap();
            assert!(!out.trim().is_empty(), "{format:?} produced nothing");
        }
    }

    #[test]
    fn exporting_an_empty_transcript_does_not_panic() {
        let mut empty = doc();
        empty.transcript.clear();
        for format in Format::all() {
            assert!(export(&empty, format, Options::default()).is_ok());
        }
    }

    #[test]
    fn subtitle_times_are_formatted_to_the_millisecond() {
        assert_eq!(subtitle_time(0.0, ','), "00:00:00,000");
        assert_eq!(subtitle_time(3_725.5, '.'), "01:02:05.500");
        assert_eq!(subtitle_time(-1.0, ','), "00:00:00,000");
    }
}
