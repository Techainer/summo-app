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
        /// Only meetings carrying every tag named. Repeat with commas: `--tag a,b`.
        #[arg(long)]
        tag: Option<String>,
        /// Only meetings with this colour. A palette name, or a hex that snaps to one.
        #[arg(long)]
        colour: Option<String>,
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

    /// Colour a meeting. With no colour, clears it.
    Colour {
        id: String,
        /// A palette name, or a hex that snaps to one. Naming a colour that is not in the palette
        /// fails with the list of ones that are.
        colour: Option<String>,
    },

    /// Rename a meeting.
    Rename { id: String, title: String },

    /// Move a meeting and its audio to the vault's trash. Nothing is deleted.
    Rm { id: String },

    /// Counters for the whole vault.
    Stats,

    /// What Summo is using on disk.
    Storage {
        /// Delete recordings older than the retention setting, and audio with no meeting left.
        #[arg(long)]
        prune: bool,
        /// Show what pruning would remove without removing it.
        #[arg(long)]
        dry_run: bool,
    },

    /// Delete one meeting's audio, keeping its transcript.
    ForgetAudio { id: String },

    /// What a day contained: hours, who with, and what is still open.
    ///
    /// No model runs — this is arithmetic over the vault, so it works offline and costs nothing.
    Report {
        /// `YYYY-MM-DD`, or `today` / `yesterday`. Defaults to today.
        #[arg(default_value = "today")]
        day: String,
        /// Report a whole ISO week ending on `day` instead of a single day.
        #[arg(long)]
        week: bool,
    },
}

pub fn run(paths: &Paths, cmd: MeetingCmd) -> Result<()> {
    let library = Library::new(paths.clone());
    match cmd {
        MeetingCmd::Ls {
            group,
            folder,
            tag,
            colour,
            person,
            from,
            to,
            without_summary,
        } => {
            let query = LibraryQuery {
                token: None,
                group: parse_group(&group)?,
                folder,
                unfiled: false,
                tag,
                colour,
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
            if let Some(colour) = detail.summary.color {
                println!("colour: {colour}");
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

        MeetingCmd::Colour { id, colour } => {
            let set = library.set_colour(&MeetingId::from(id), colour.as_deref())?;
            println!("colour: {}", set.unwrap_or("none"));
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

        MeetingCmd::Storage { prune, dry_run } => {
            let usage = summo_vault::storage::usage(paths)?;
            let mb = summo_vault::human_bytes;
            println!("transcripts  {:>10}", mb(usage.vault_bytes));
            println!("recordings   {:>10}", mb(usage.audio_bytes));
            println!("models       {:>10}", mb(usage.model_bytes));
            println!("total        {:>10}", mb(usage.total_bytes));

            if !usage.recordings.is_empty() {
                println!("\nlargest recordings");
                for r in usage.recordings.iter().take(10) {
                    println!(
                        "  {:>9}  {}  {}",
                        mb(r.bytes),
                        r.day,
                        truncate(&r.title, 40)
                    );
                }
            }
            if !usage.orphaned.is_empty() {
                println!(
                    "\n{} recording(s) with no meeting left, {} — `--prune` reclaims them",
                    usage.orphaned.len(),
                    mb(usage.orphaned.iter().map(|r| r.bytes).sum())
                );
            }

            if prune || dry_run {
                let settings = summo_core::Settings::load(&paths.settings())?;
                let today = now().date().to_string();
                let pruned = summo_vault::storage::prune(
                    paths,
                    settings.storage.audio_retention_days,
                    &today,
                    dry_run,
                )?;
                println!(
                    "\n{} {} recording(s), {}",
                    if dry_run { "would remove" } else { "removed" },
                    pruned.removed.len(),
                    mb(pruned.freed_bytes)
                );
                for r in &pruned.removed {
                    // An orphan has no meeting to name it; saying so beats printing a blank line
                    // that reads like a bug.
                    if r.title.is_empty() {
                        println!("  {:<10} (cuộc họp đã bị xoá) {}", "", r.id.as_str());
                    } else {
                        println!("  {} {}", r.day, truncate(&r.title, 48));
                    }
                }
                if settings.storage.audio_retention_days == 0 {
                    println!(
                        "retention is off (storage.audio_retention_days = 0), so nothing ages out"
                    );
                }
            }
            Ok(())
        }

        MeetingCmd::ForgetAudio { id } => {
            let freed = summo_vault::storage::forget_audio(paths, &MeetingId::from(id))?;
            println!("freed {}", summo_vault::human_bytes(freed));
            Ok(())
        }

        MeetingCmd::Report { day, week } => {
            let today = now().date().to_string();
            let to = match day.as_str() {
                "today" => today,
                "yesterday" => shift(&today, -1),
                explicit => explicit.to_string(),
            };
            let from = if week { shift(&to, -6) } else { to.clone() };
            print_report(&summo_vault::report::between(&paths.vault(), &from, &to)?);
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
/// Shift a `YYYY-MM-DD` by whole days, leaving it alone if it will not parse.
fn shift(day: &str, days: i64) -> String {
    let Ok(date) = time::Date::parse(day, &time::format_description::well_known::Iso8601::DATE)
    else {
        return day.to_string();
    };
    (date + time::Duration::days(days)).to_string()
}

fn print_report(report: &summo_vault::report::Report) {
    if report.from == report.to {
        println!("{}", report.from);
    } else {
        println!("{} → {}", report.from, report.to);
    }

    if report.meetings.is_empty() {
        println!("\nKhông có buổi họp nào.");
        return;
    }

    println!(
        "\n{} buổi họp · {}",
        report.meetings.len(),
        duration(report.total_seconds)
    );
    for meeting in &report.meetings {
        println!(
            "  {:>6}  {}{}",
            duration(meeting.duration),
            meeting.title,
            if meeting.has_summary {
                ""
            } else {
                "  (chưa tóm tắt)"
            }
        );
    }

    if !report.people.is_empty() {
        println!("\nThời gian với ai");
        for person in report.people.iter().take(8) {
            println!("  {:>6}  {}", duration(person.seconds), person.name);
        }
    }

    if !report.open_actions.is_empty() {
        // The only part of a vault that decays: a task nobody looked at again.
        println!("\nCòn phải làm ({})", report.open_actions.len());
        for action in &report.open_actions {
            println!("  [ ] {}  — {}", action.text, action.meeting_title);
        }
    }
    if report.done_actions > 0 {
        println!("\nĐã xong: {}", report.done_actions);
    }

    if !report.quiet_days.is_empty() && report.from != report.to {
        println!("\nKhông họp: {}", report.quiet_days.join(", "));
    }
}

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
            kind: summo_vault::index::Kind::Meeting,
            id: MeetingId::from("01A".to_string()),
            title: title.to_string(),
            folder: String::new(),
            date: "2026-08-09T10:00:00+07:00".into(),
            day: "2026-08-09".into(),
            duration: 600,
            participants: Vec::new(),
            tags: Vec::new(),
            color: None,
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
