//! The user's calendars, as the app sees them.
//!
//! Every `.ics` in `~/.summo/calendars/` is read on demand rather than cached: these are small
//! files, a subscription rewrites the whole file when it syncs, and a cache would mean the app
//! showing a meeting that was cancelled an hour ago.
//!
//! **Nothing here starts a recording.** The strongest thing this does is say which event a
//! recording probably belongs to, so a meeting note can be titled after it. Auto-recording from a
//! calendar is how an app records the therapy appointment somebody put in their work calendar, and
//! `Recording::suggest_on_meeting` stays a suggestion.

use serde::Serialize;
use summo_calendar::{Confidence, Event};
use summo_core::{Result, paths::Paths};

/// One event, flattened for the interface.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub uid: String,
    pub summary: String,
    /// `YYYY-MM-DD` in the event's own day, which is the day a person names it by.
    pub day: String,
    /// Seconds since the epoch. Approximate for an event whose `TZID` is unresolved — see
    /// `summo_calendar::When::approx_epoch`.
    pub start_epoch: i64,
    pub duration_s: Option<i64>,
    pub location: Option<String>,
    pub conference: Option<String>,
    pub attendees: Vec<String>,
    /// Whether it repeats. The rule is not expanded, so an occurrence other than the one written in
    /// the file will not appear — flagged rather than silently missing.
    pub repeats: bool,
    /// Which calendar file it came from, so a user with three subscriptions can tell them apart.
    pub calendar: String,
}

impl Entry {
    fn from(event: &Event, calendar: &str) -> Option<Self> {
        let start = event.start.as_ref()?;
        Some(Self {
            uid: event.uid.clone(),
            summary: event.summary.clone(),
            day: start.day(),
            start_epoch: start.approx_epoch(),
            duration_s: event.duration_s(),
            location: event.location.clone(),
            conference: event.conference.clone(),
            attendees: event.attendees.clone(),
            repeats: event.rrule.is_some(),
            calendar: calendar.to_string(),
        })
    }
}

/// Read every calendar the user has installed.
///
/// A broken file costs that calendar, not the screen: these are downloaded from servers that
/// occasionally serve an error page with a `.ics` extension, and one of those must not take the
/// agenda down.
#[must_use]
pub fn load(paths: &Paths) -> Vec<(String, Vec<Event>)> {
    let dir = paths.calendars();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No directory is the normal case for a user who has not added one.
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ics") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();

        match summo_calendar::read(&path) {
            Ok(events) => out.push((name, events)),
            Err(e) => tracing::warn!(file = %path.display(), error = %e, "skipping a calendar"),
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Meetings worth showing, in time order.
///
/// Filtered to things that look like meetings, because an agenda that lists focus blocks and
/// birthdays is an agenda nobody reads.
#[must_use]
pub fn agenda(paths: &Paths) -> Vec<Entry> {
    let mut out: Vec<Entry> = load(paths)
        .iter()
        .flat_map(|(name, events)| {
            summo_calendar::meetings(events)
                .into_iter()
                .filter_map(|e| Entry::from(e, name))
        })
        .collect();
    out.sort_by_key(|e| e.start_epoch);
    out
}

/// What a recording that started at `started_epoch` was probably for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub entry: Entry,
    /// `inside` or `near`.
    pub confidence: &'static str,
    pub offset_s: i64,
}

/// Suggest a title for a recording, or nothing.
///
/// Nothing is the right answer surprisingly often, and it is better than a wrong one: a meeting note
/// titled after the wrong event has to be noticed before it can be corrected, and the transcript
/// underneath it looks like a mistake rather than a mislabel.
#[must_use]
pub fn suggest(paths: &Paths, started_epoch: i64) -> Option<Suggestion> {
    let calendars = load(paths);
    let mut best: Option<Suggestion> = None;

    for (name, events) in &calendars {
        let Some(found) = summo_calendar::best_match(events, started_epoch) else {
            continue;
        };
        let Some(entry) = Entry::from(found.event, name) else {
            continue;
        };
        let candidate = Suggestion {
            entry,
            confidence: match found.confidence {
                Confidence::Inside => "inside",
                Confidence::Near => "near",
            },
            offset_s: found.offset_s,
        };

        // Across calendars, the same rule as within one: an event in progress beats one nearby, and
        // ties break on distance.
        let better = match &best {
            None => true,
            Some(current) => {
                let rank = |c: &str| u8::from(c != "inside");
                (rank(candidate.confidence), candidate.offset_s.abs())
                    < (rank(current.confidence), current.offset_s.abs())
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}

/// Install a calendar file the user picked, copying it into the app's own directory.
///
/// Copied rather than referenced: a path into `~/Downloads` breaks the moment the user tidies up,
/// and a calendar that silently stops updating is worse than one that was never added.
pub fn install(paths: &Paths, source: &std::path::Path, name: &str) -> Result<std::path::PathBuf> {
    let name = safe_name(name);
    if name.is_empty() {
        return Err(summo_core::Error::Other("lịch cần có tên".into()));
    }

    // Parse before installing: a file that is not a calendar should be refused now, not discovered
    // as an empty agenda later.
    let events = summo_calendar::read(source)?;
    if events.is_empty() {
        return Err(summo_core::Error::Other(format!(
            "{} không có sự kiện nào",
            source.display()
        )));
    }

    let dir = paths.calendars();
    std::fs::create_dir_all(&dir).map_err(|e| summo_core::Error::io(&dir, e))?;
    let target = dir.join(format!("{name}.ics"));
    std::fs::copy(source, &target).map_err(|e| summo_core::Error::io(&target, e))?;
    Ok(target)
}

/// A calendar name safe to put in a filename.
///
/// The name arrives from an HTTP body, and this becomes a path. Letters, digits, hyphens and
/// underscores only — `../../settings` would otherwise write outside the directory.
#[must_use]
pub fn safe_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect()
}

/// Remove a calendar. `false` when there was nothing to remove.
pub fn forget(paths: &Paths, name: &str) -> Result<bool> {
    let name = safe_name(name);
    if name.is_empty() {
        return Ok(false);
    }
    let path = paths.calendars().join(format!("{name}.ics"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(summo_core::Error::io(&path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEETING: &str = "BEGIN:VEVENT\r\nUID:standup\r\n\
DTSTART:20260810T090000Z\r\nDTEND:20260810T093000Z\r\nSUMMARY:Standup\r\n\
ATTENDEE:mailto:a@x\r\nATTENDEE:mailto:b@x\r\nEND:VEVENT\r\n";

    fn epoch(hh: u8, mm: u8) -> i64 {
        summo_calendar::When::Utc {
            y: 2026,
            m: 8,
            d: 10,
            hh,
            mm,
            ss: 0,
        }
        .approx_epoch()
    }

    fn with(files: &[(&str, &str)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.calendars()).unwrap();
        for (name, body) in files {
            std::fs::write(paths.calendars().join(name), body).unwrap();
        }
        (tmp, paths)
    }

    #[test]
    fn no_calendars_is_an_empty_agenda_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(agenda(&Paths::at(tmp.path())).is_empty());
    }

    #[test]
    fn an_installed_calendar_shows_its_meetings() {
        let (_tmp, paths) = with(&[("work.ics", MEETING)]);
        let entries = agenda(&paths);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].summary, "Standup");
        assert_eq!(entries[0].calendar, "work");
        assert_eq!(entries[0].duration_s, Some(1_800));
    }

    /// These files come from servers that occasionally serve an error page with a `.ics`
    /// extension. One of those must not take the agenda down.
    #[test]
    fn a_broken_calendar_costs_itself_and_not_the_others() {
        let (_tmp, paths) = with(&[("bad.ics", "<html>not a calendar</html>"), ("work.ics", MEETING)]);
        let entries = agenda(&paths);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].calendar, "work");
    }

    #[test]
    fn files_that_are_not_calendars_are_ignored() {
        let (_tmp, paths) = with(&[("notes.txt", MEETING)]);
        assert!(agenda(&paths).is_empty());
    }

    #[test]
    fn meetings_from_several_calendars_interleave_by_time() {
        let later = MEETING
            .replace("UID:standup", "UID:later")
            .replace("T090000Z", "T140000Z")
            .replace("T093000Z", "T150000Z")
            .replace("SUMMARY:Standup", "SUMMARY:Later");
        let (_tmp, paths) = with(&[("a.ics", &later), ("b.ics", MEETING)]);

        let summaries: Vec<_> = agenda(&paths).into_iter().map(|e| e.summary).collect();
        assert_eq!(summaries, ["Standup", "Later"]);
    }

    #[test]
    fn a_recording_during_a_meeting_is_suggested_that_meeting() {
        let (_tmp, paths) = with(&[("work.ics", MEETING)]);
        let s = suggest(&paths, epoch(9, 10)).unwrap();
        assert_eq!(s.entry.summary, "Standup");
        assert_eq!(s.confidence, "inside");
    }

    /// A wrongly titled meeting note has to be noticed before it can be corrected, and the
    /// transcript under it reads as a mistake rather than a mislabel.
    #[test]
    fn a_recording_at_an_unrelated_time_is_suggested_nothing() {
        let (_tmp, paths) = with(&[("work.ics", MEETING)]);
        assert!(suggest(&paths, epoch(18, 0)).is_none());
    }

    #[test]
    fn an_event_in_progress_wins_across_calendars_too() {
        let soon = MEETING
            .replace("UID:standup", "UID:soon")
            .replace("T090000Z", "T093000Z")
            .replace("T093000Z\r\nSUMMARY", "T100000Z\r\nSUMMARY")
            .replace("SUMMARY:Standup", "SUMMARY:Soon");
        let (_tmp, paths) = with(&[("a.ics", MEETING), ("b.ics", &soon)]);

        let s = suggest(&paths, epoch(9, 28)).unwrap();
        assert_eq!(s.entry.summary, "Standup", "the one already running");
    }

    // The name arrives from an HTTP body and becomes a path.
    #[test]
    fn a_calendar_name_cannot_escape_the_directory() {
        assert_eq!(safe_name("../../settings"), "settings");
        assert_eq!(safe_name("work"), "work");
        assert_eq!(safe_name("công việc"), "cngvic");
        assert_eq!(safe_name("///"), "");
    }

    #[test]
    fn installing_copies_the_file_so_it_survives_a_tidy_up() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let source = tmp.path().join("downloaded.ics");
        std::fs::write(&source, MEETING).unwrap();

        let installed = install(&paths, &source, "work").unwrap();
        std::fs::remove_file(&source).unwrap();

        assert!(installed.exists(), "the copy survives the original");
        assert_eq!(agenda(&paths).len(), 1);
    }

    /// A file that is not a calendar should be refused when it is added, not discovered as an empty
    /// agenda a week later.
    #[test]
    fn installing_something_that_is_not_a_calendar_is_refused_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let source = tmp.path().join("nope.ics");
        std::fs::write(&source, "<html>error</html>").unwrap();

        let err = install(&paths, &source, "work").unwrap_err().to_string();
        assert!(err.contains("không có sự kiện"), "{err}");
        assert!(agenda(&paths).is_empty());
    }

    #[test]
    fn installing_without_a_usable_name_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let source = tmp.path().join("c.ics");
        std::fs::write(&source, MEETING).unwrap();
        assert!(install(&paths, &source, "///").is_err());
    }

    #[test]
    fn forgetting_a_calendar_that_is_not_there_is_not_an_error() {
        let (_tmp, paths) = with(&[("work.ics", MEETING)]);
        assert!(!forget(&paths, "personal").unwrap());
        assert!(forget(&paths, "work").unwrap());
        assert!(agenda(&paths).is_empty());
    }

    #[test]
    fn a_repeating_meeting_is_flagged_rather_than_silently_showing_one_occurrence() {
        let repeating = MEETING.replace(
            "SUMMARY:Standup",
            "SUMMARY:Standup\r\nRRULE:FREQ=WEEKLY;BYDAY=MO",
        );
        let (_tmp, paths) = with(&[("work.ics", &repeating)]);
        assert!(agenda(&paths)[0].repeats);
    }
}
