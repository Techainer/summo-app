//! `summo meetings` — the library from a terminal.
//!
//! The same scan the app uses, printed. This exists because the vault is a folder of files: a user
//! who wants to know what is in it should not have to open a GUI, and anything the app can do to a
//! meeting should be scriptable.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use summo_core::{MeetingId, paths::Paths};
use summo_vault::{
    Library, LibraryQuery, MeetingSummary,
    library::{GroupBy, SummaryGroup},
};
use time::OffsetDateTime;

#[derive(Subcommand)]
pub enum MeetingCmd {
    /// List meetings, newest first.
    Ls {
        /// `day`, `week`, `folder` or `none`.
        #[arg(long, default_value = "day")]
        group: String,
        /// Only meetings in this folder, including its subfolders.
        #[arg(long)]
        folder: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Only meetings this person took part in.
        #[arg(long)]
        person: Option<String>,
        /// Inclusive `YYYY-MM-DD` bounds.
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Only meetings that have not been summarised yet.
        #[arg(long)]
        without_summary: bool,
    },

    /// Search every meeting, ignoring Vietnamese tone marks.
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Print one meeting.
    Show { id: String },

    /// Move a meeting into a folder. An empty name moves it back to the top level.
    Mv { id: String, folder: String },

    /// Replace a meeting's tags.
    Tag { id: String, tags: Vec<String> },

    /// Rename a meeting.
    Rename { id: String, title: String },

    /// Move a meeting and its audio to the vault's trash. Nothing is deleted.
    Rm { id: String },

    /// Counters for the whole vault.
    Stats,
}

pub fn run(paths: &Paths, cmd: MeetingCmd) -> Result<()> {
    let library = Library::new(paths.clone());
    match cmd {
        MeetingCmd::Ls {
            group,
            folder,
            tag,
            person,
            from,
            to,
            without_summary,
        } => {
            let query = LibraryQuery {
                token: None,
                group: parse_group(&group)?,
                folder,
                tag,
                person,
                from,
                to,
                without_summary,
            };
            let view = library.view(&query, now())?;
            if view.total == 0 {
                println!("no meetings match");
            }
            for group in &view.groups {
                print_group(group);
            }
            // A file that would not parse is the one thing a listing must never hide: the meeting
            // is on disk, and the user needs to know why it is not on screen.
            for skipped in &view.skipped {
                eprintln!("skipped {}: {}", skipped.path.display(), skipped.reason);
            }
            Ok(())
        }

        MeetingCmd::Search { query, limit } => {
            let query = query.join(" ");
            let hits = library.search(&query, limit)?;
            if hits.is_empty() {
                println!("nothing matched {query:?}");
            }
            for hit in hits {
                println!(
                    "{}  {}  ({} match{})",
                    hit.meeting.id.as_str(),
                    hit.meeting.title,
                    hit.matches,
                    if hit.matches == 1 { "" } else { "es" }
                );
                for excerpt in hit.excerpts {
                    let at = excerpt
                        .t0
                        .map(|t| format!("[{}] ", summo_vault::meeting::format_timestamp(t)))
                        .unwrap_or_default();
                    let who = excerpt
                        .speaker
                        .map(|s| format!("{s} — "))
                        .unwrap_or_default();
                    println!("    {at}{who}{}", excerpt.text);
                }
            }
            Ok(())
        }

        MeetingCmd::Show { id } => {
            let detail = library
                .detail(&MeetingId::from(id.clone()))
                .with_context(|| format!("cannot show meeting {id}"))?;
            println!("# {}", detail.summary.title);
            println!(
                "{}  ·  {}  ·  {}",
                detail.summary.date,
                duration(detail.summary.duration),
                if detail.summary.folder.is_empty() {
                    "top level".to_string()
                } else {
                    detail.summary.folder.clone()
                }
            );
            if !detail.summary.tags.is_empty() {
                println!("tags: {}", detail.summary.tags.join(", "));
            }
            if !detail.summary.participants.is_empty() {
                println!("with: {}", detail.summary.participants.join(", "));
            }
            for section in &detail.sections {
                println!("\n## {}\n{}", section.heading, section.body);
            }
            println!("\n## Transcript ({} lines)", detail.transcript.len());
            for segment in &detail.transcript {
                println!(
                    "[{}] {} — {}",
                    summo_vault::meeting::format_timestamp(segment.t0),
                    segment
                        .speaker
                        .as_ref()
                        .map_or("?", summo_core::SpeakerId::as_str),
                    segment.text
                );
            }
            Ok(())
        }

        MeetingCmd::Mv { id, folder } => {
            let path = library.move_to_folder(&MeetingId::from(id), &folder)?;
            println!("moved to {}", path.display());
            Ok(())
        }

        MeetingCmd::Tag { id, tags } => {
            let tags = library.set_tags(&MeetingId::from(id), tags)?;
            println!(
                "tags: {}",
                if tags.is_empty() {
                    "none".to_string()
                } else {
                    tags.join(", ")
                }
            );
            Ok(())
        }

        MeetingCmd::Rename { id, title } => {
            let title = library.rename(&MeetingId::from(id), &title)?;
            println!("renamed to {title}");
            Ok(())
        }

        MeetingCmd::Rm { id } => {
            let path = library.trash(&MeetingId::from(id))?;
            println!("moved to {}", path.display());
            println!("nothing was deleted — remove that file yourself if you are sure");
            Ok(())
        }

        MeetingCmd::Stats => {
            let index = library.scan()?;
            let stats = index.stats(now());
            println!("meetings          {}", stats.meetings);
            println!("recorded          {}", duration(stats.total_duration));
            println!(
                "last seven days   {} ({})",
                stats.last_seven_days,
                duration(stats.last_seven_days_duration)
            );
            println!("people            {}", stats.people);
            println!("tags              {}", stats.tags);
            println!("without a summary {}", stats.without_summary);
            if let Some(latest) = stats.latest {
                println!("most recent       {latest}");
            }
            Ok(())
        }
    }
}

fn print_group(group: &SummaryGroup) {
    if group.key.is_empty() {
        for meeting in &group.meetings {
            println!("{}", line(meeting));
        }
        return;
    }
    println!("\n{}", group.key);
    for meeting in &group.meetings {
        println!("  {}", line(meeting));
    }
}

fn line(m: &MeetingSummary) -> String {
    /// Column width for titles.
    const TITLE: usize = 40;

    let time = m.date.get(11..16).unwrap_or("     ");
    let mark = if m.has_summary { " " } else { "·" };
    // Padded by characters, not bytes: `{:<40}` counts bytes, so a Vietnamese title would be
    // padded short and the columns to its right would not line up.
    let title = truncate(&m.title, TITLE);
    let pad = " ".repeat(TITLE.saturating_sub(title.chars().count()));
    format!(
        "{time} {mark} {title}{pad} {:>8}  {}",
        duration(m.duration),
        m.id.as_str()
    )
}

/// Truncate on character boundaries — Vietnamese text is not one byte per column.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn duration(secs: u64) -> String {
    match secs {
        0 => "—".to_string(),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h{:02}", s / 3600, (s % 3600) / 60),
    }
}

fn parse_group(name: &str) -> Result<GroupBy> {
    Ok(match name {
        "day" => GroupBy::Day,
        "week" => GroupBy::Week,
        "folder" => GroupBy::Folder,
        "none" | "flat" => GroupBy::None,
        other => bail!("unknown grouping `{other}`. Use day, week, folder or none."),
    })
}

/// Local time, falling back to UTC where the OS will not report an offset.
///
/// `now_local` fails in a multi-threaded process on some Unixes; a wrong "last seven days" boundary
/// by a few hours is better than refusing to print the library.
fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_truncate_on_character_boundaries() {
        // Byte slicing would panic here, and Vietnamese titles are full of multi-byte characters.
        let title = "Họp đánh giá quý ba với khách hàng chiến lược";
        let out = truncate(title, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn columns_line_up_when_a_title_is_not_ascii() {
        let make = |title: &str| MeetingSummary {
            id: MeetingId::from("01A".to_string()),
            title: title.to_string(),
            folder: String::new(),
            date: "2026-08-09T10:00:00+07:00".into(),
            day: "2026-08-09".into(),
            duration: 600,
            participants: Vec::new(),
            tags: Vec::new(),
            has_summary: true,
            size_bytes: 0,
            file: "a.md".into(),
        };
        let ascii = line(&make("Weekly Sync"));
        let viet = line(&make("Demo khách hàng ACME"));
        assert_eq!(
            ascii.chars().count(),
            viet.chars().count(),
            "\n{ascii}\n{viet}"
        );
    }

    #[test]
    fn durations_read_as_time_not_seconds() {
        assert_eq!(duration(0), "—");
        assert_eq!(duration(600), "10m");
        assert_eq!(duration(5_400), "1h30");
    }

    #[test]
    fn an_unknown_grouping_is_refused_rather_than_defaulted() {
        assert!(parse_group("day").is_ok());
        assert!(parse_group("month").is_err());
    }
}
