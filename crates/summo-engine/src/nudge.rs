//! When the agent is allowed to interrupt.
//!
//! An assistant that only answers when spoken to is a search box. One that pings all day is
//! uninstalled in a week. The whole design question is *when*, and this module is the answer being
//! written down rather than scattered through the code that fires notifications.
//!
//! Four things are worth interrupting for, and nothing else is:
//!
//! * **The end of the day.** A report of what happened and what is still open — the one moment a
//!   person is willing to look back rather than forward.
//! * **A draft nobody read.** The meeting is over, the summary is written, and it is sitting there
//!   unconfirmed.
//! * **A task past its due date.** Once, when it becomes overdue. Not every day after.
//! * **Monday morning.** What last week contained.
//!
//! Two rules keep it from becoming noise:
//!
//! * **Once per thing per day.** State lives in `~/.summo/nudges.json`, so restarting the daemon
//!   does not re-fire everything it already said.
//! * **Never while recording.** A notification during a meeting is the worst possible moment, and
//!   it is also the moment the user is most likely to be sharing their screen.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};

/// Why the agent is speaking up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// End of day: what happened, what is still open.
    DailyReport,
    /// Start of the week: what last week contained.
    WeeklyRollup,
    /// A summary was written and never looked at.
    DraftWaiting,
    /// A task passed its due date.
    Overdue,
    /// A meeting on the calendar is starting, and nothing is recording it.
    MeetingSoon,
}

impl Reason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DailyReport => "daily-report",
            Self::WeeklyRollup => "weekly-rollup",
            Self::DraftWaiting => "draft-waiting",
            Self::Overdue => "overdue",
            Self::MeetingSoon => "meeting-soon",
        }
    }
}

/// Something to tell the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nudge {
    pub reason: Reason,
    pub title: String,
    pub body: String,
    /// Where tapping it should go, e.g. `/analytics` or `/meetings/01A`.
    pub route: String,
    /// Key that makes this nudge unique for the day, so it fires once.
    pub key: String,
}

/// What has already been said, by key, to the day it was said on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    fired: BTreeMap<String, String>,
}

impl Seen {
    fn already_today(&self, key: &str, today: &str) -> bool {
        self.fired.get(key).is_some_and(|day| day == today)
    }

    fn record(&mut self, key: &str, today: &str) {
        self.fired.insert(key.to_string(), today.to_string());
    }

    /// Forget entries older than `today` by more than a week, so the file does not grow forever.
    fn prune(&mut self, today: &str) {
        let cutoff = shift(today, -7);
        self.fired.retain(|_, day| day.as_str() >= cutoff.as_str());
    }
}

/// How far back to look for work that is still open, in days.
///
/// A task nobody touched in three months is not something a notification will fix; it is something
/// to close. Bounding the window also bounds the scan.
const OVERDUE_WINDOW: i64 = 90;

fn path(paths: &Paths) -> std::path::PathBuf {
    paths.root().join("nudges.json")
}

pub fn load(paths: &Paths) -> Result<Seen> {
    match std::fs::read_to_string(path(paths)) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Seen::default()),
        Err(e) => Err(Error::io(path(paths), e)),
    }
}

pub fn save(paths: &Paths, seen: &Seen) -> Result<()> {
    summo_vault::write::write_atomically(&path(paths), serde_json::to_vec_pretty(seen)?.as_slice())
}

/// What the daemon should say right now, given the clock and what it has already said.
///
/// `now_hour` is local, 0–23. `weekday` is 1 for Monday through 7 for Sunday, matching ISO.
/// Passed in rather than read from the clock so the decision is testable — this is the part that
/// gets it wrong at 23:59 on a Sunday, and a test is the only way to find out.
pub fn due(
    paths: &Paths,
    seen: &Seen,
    today: &str,
    now_hour: u8,
    weekday: u8,
    recording: bool,
) -> Result<Vec<Nudge>> {
    // The worst possible moment, and the one where a screen is most likely being shared.
    if recording {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();

    // A draft nobody read. Immediate — the meeting just ended and it is on their mind.
    for meeting in unread_drafts(paths)? {
        let key = format!("draft:{meeting}");
        if seen.already_today(&key, today) {
            continue;
        }
        out.push(Nudge {
            reason: Reason::DraftWaiting,
            title: "Bản tóm tắt đang chờ bạn".into(),
            body: "Họp xong rồi, xem qua rồi xác nhận nhé.".into(),
            route: format!("/meetings/{meeting}"),
            key,
        });
    }

    // Everything below is a look back, and only after the working day.
    if now_hour < 17 {
        return Ok(out);
    }

    let report = summo_vault::report::day(&paths.vault(), today)?;
    if !report.meetings.is_empty() {
        let key = format!("daily:{today}");
        if !seen.already_today(&key, today) {
            out.push(Nudge {
                reason: Reason::DailyReport,
                title: format!("Hôm nay: {} buổi họp", report.meetings.len()),
                body: summary_line(&report),
                route: "/analytics".into(),
                key,
            });
        }
    }

    // Overdue work, as one nudge rather than one per task — five notifications about five tasks is
    // four too many.
    //
    // Read over a window rather than out of today's report: a task is overdue *because* it came
    // from an older meeting, so today's report is the one place it will never appear.
    let history =
        summo_vault::report::between(&paths.vault(), &shift(today, -OVERDUE_WINDOW), today)?;
    let overdue: Vec<_> = history
        .open_actions
        .iter()
        .filter(|a| a.day.as_str() < today)
        .collect();
    if !overdue.is_empty() {
        let key = format!("overdue:{today}");
        if !seen.already_today(&key, today) {
            out.push(Nudge {
                reason: Reason::Overdue,
                title: format!("{} việc còn treo", overdue.len()),
                body: overdue
                    .iter()
                    .take(3)
                    .map(|a| a.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
                route: "/tasks".into(),
                key,
            });
        }
    }

    // Monday, looking back at last week.
    if weekday == 1 {
        let key = format!("weekly:{today}");
        if !seen.already_today(&key, today) {
            out.push(Nudge {
                reason: Reason::WeeklyRollup,
                title: "Tuần trước của bạn".into(),
                body: "Xem lại thời gian họp và việc còn lại.".into(),
                route: "/analytics".into(),
                key,
            });
        }
    }

    Ok(out)
}

/// How early a meeting is worth mentioning, and how late it is still worth mentioning it.
///
/// Five minutes before, because the useful moment is while the person is opening the call, not
/// while they are already in it. Ten minutes after, because the far more common failure is
/// remembering to record at minute eight — and a prompt then still saves the rest of the meeting.
const BEFORE_S: i64 = 300;
const AFTER_S: i64 = 600;

/// Ask whether to take notes, because a meeting on the calendar is starting.
///
/// Separate from [`due`] because it is a different question asked on a different clock: `due` looks
/// back over a day at what the vault holds, this looks at the next few minutes of somebody else's
/// calendar. Folding them together would mean either running the calendar check hourly or running
/// the vault scan every minute.
///
/// **It only ever asks.** `suggest` is `Recording::suggest_on_meeting` and this returns nothing when
/// it is off. Nothing here starts a recording, and nothing here should ever be changed to: a
/// calendar entry is not consent, and the therapy appointment in a work calendar is the reason.
pub fn meeting_soon(
    paths: &Paths,
    seen: &Seen,
    today: &str,
    now_epoch: i64,
    recording: bool,
    suggest: bool,
) -> Vec<Nudge> {
    if !suggest || recording {
        return Vec::new();
    }

    let mut out = Vec::new();
    for entry in crate::agenda::agenda(paths) {
        let until = entry.start_epoch - now_epoch;
        if until > BEFORE_S || -until > AFTER_S {
            continue;
        }
        // Keyed by the occurrence, not by the event: a daily standup is a different meeting every
        // morning, and one that fired on Monday must still fire on Tuesday.
        let key = format!("meeting:{}:{}", entry.uid, entry.start_epoch);
        if seen.already_today(&key, today) {
            continue;
        }
        out.push(Nudge {
            reason: Reason::MeetingSoon,
            title: if until > 0 {
                format!("{} sắp bắt đầu", entry.summary)
            } else {
                format!("{} đang diễn ra", entry.summary)
            },
            body: "Ghi chú buổi này không?".into(),
            route: "/".into(),
            key,
        });
    }
    // One prompt, even when two calendars hold the same meeting or two meetings collide.
    out.truncate(1);
    out
}

/// Mark nudges as said, so they do not fire again today.
pub fn record(paths: &Paths, seen: &mut Seen, nudges: &[Nudge], today: &str) -> Result<()> {
    for nudge in nudges {
        seen.record(&nudge.key, today);
    }
    seen.prune(today);
    save(paths, seen)
}

fn summary_line(report: &summo_vault::report::Report) -> String {
    let hours = report.total_seconds / 3600;
    let minutes = (report.total_seconds % 3600) / 60;
    let time = if hours > 0 {
        format!("{hours} giờ {minutes} phút")
    } else {
        format!("{minutes} phút")
    };
    if report.open_actions.is_empty() {
        format!("{time} họp, không còn việc treo.")
    } else {
        format!("{time} họp, {} việc còn lại.", report.open_actions.len())
    }
}

/// Meetings whose summary was written but never confirmed.
///
/// Reads the notes, not the `drafts/` sidecar. The sidecar holds the refinement conversation; a
/// draft that was generated and never discussed has no sidecar at all, and looking there was a bug
/// that made exactly the case worth nudging about invisible.
fn unread_drafts(paths: &Paths) -> Result<Vec<String>> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;

    let mut out = Vec::new();
    for entry in index.entries() {
        let Ok(body) = std::fs::read_to_string(vault.join(&entry.path)) else {
            continue;
        };
        if body.contains(summo_vault::pending::MARKER) {
            out.push(entry.id.to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// `YYYY-MM-DD` shifted by whole days.
fn shift(day: &str, days: i64) -> String {
    let Ok(date) = time::Date::parse(day, &time::format_description::well_known::Iso8601::DATE)
    else {
        return day.to_string();
    };
    (date + time::Duration::days(days)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A calendar with one meeting in it, starting at `start_epoch`.
    fn calendar(paths: &Paths, start_epoch: i64) {
        let dir = paths.calendars();
        std::fs::create_dir_all(&dir).unwrap();
        let stamp = |epoch: i64| {
            time::OffsetDateTime::from_unix_timestamp(epoch)
                .unwrap()
                .format(
                    &time::format_description::parse_borrowed::<2>(
                        "[year][month][day]T[hour][minute][second]Z",
                    )
                    .unwrap(),
                )
                .unwrap()
        };
        std::fs::write(
            dir.join("work.ics"),
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:standup-1\r\nSUMMARY:Họp đầu tuần\r\n\
                 DTSTART:{}\r\nDTEND:{}\r\nATTENDEE:mailto:a@x.vn\r\nATTENDEE:mailto:b@x.vn\r\n\
                 END:VEVENT\r\nEND:VCALENDAR\r\n",
                stamp(start_epoch),
                stamp(start_epoch + 1800),
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_meeting_about_to_start_is_worth_asking_about() {
        let (_dir, paths) = vault(&[]);
        let now = 1_800_000_000;
        calendar(&paths, now + 120);

        let due = meeting_soon(&paths, &Seen::default(), "2027-01-15", now, false, true);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].reason, Reason::MeetingSoon);
        assert!(due[0].title.contains("Họp đầu tuần"));
    }

    /// The switch is `Recording::suggest_on_meeting`, and off means silent — not quieter.
    #[test]
    fn nothing_is_said_when_the_suggestion_is_turned_off() {
        let (_dir, paths) = vault(&[]);
        let now = 1_800_000_000;
        calendar(&paths, now + 120);

        assert!(meeting_soon(&paths, &Seen::default(), "2027-01-15", now, false, false).is_empty());
        // Nor during a meeting that is already being recorded, which is the worst moment for a
        // notification and the one where a screen is most likely shared.
        assert!(meeting_soon(&paths, &Seen::default(), "2027-01-15", now, true, true).is_empty());
    }

    #[test]
    fn a_meeting_that_is_not_close_is_left_alone() {
        let (_dir, paths) = vault(&[]);
        let now = 1_800_000_000;
        calendar(&paths, now + 3_600);
        assert!(meeting_soon(&paths, &Seen::default(), "2027-01-15", now, false, true).is_empty());

        // And one that has been running for half an hour: whoever is in it is in it.
        let (_dir2, later) = vault(&[]);
        calendar(&later, now - 1_800);
        assert!(meeting_soon(&later, &Seen::default(), "2027-01-15", now, false, true).is_empty());
    }

    #[test]
    fn the_same_meeting_is_only_asked_about_once() {
        let (_dir, paths) = vault(&[]);
        let now = 1_800_000_000;
        calendar(&paths, now + 120);

        let mut seen = Seen::default();
        let due = meeting_soon(&paths, &seen, "2027-01-15", now, false, true);
        record(&paths, &mut seen, &due, "2027-01-15").unwrap();
        assert!(meeting_soon(&paths, &seen, "2027-01-15", now, false, true).is_empty());
    }

    fn vault(meetings: &[(&str, &str, &str)]) -> (TempDir, Paths) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        for (id, date, body) in meetings {
            std::fs::write(
                paths.meetings().join(format!("{id}.md")),
                format!(
                    "---\nid: {id}\ndate: {date}\nduration: 3600\n\
                     participants: []\ntags: []\n---\n# Họp {id}\n\n{body}"
                ),
            )
            .unwrap();
        }
        (dir, paths)
    }

    /// Mark a meeting's summary as the agent's, unapproved — the way `draft::generate` does.
    fn with_draft(paths: &Paths, id: &str) {
        let path = paths.meetings().join(format!("{id}.md"));
        let body = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            body.replace("## Tóm tắt", "## Tóm tắt <!-- summo:draft -->"),
        )
        .unwrap();
    }

    const TODAY: &str = "2026-08-10";

    /// The one rule with no exceptions.
    #[test]
    fn nothing_fires_while_recording() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        with_draft(&paths, "01A");
        let seen = Seen::default();
        let nudges = due(&paths, &seen, TODAY, 18, 1, true).expect("due");
        assert!(nudges.is_empty(), "{nudges:?}");
    }

    #[test]
    fn a_draft_nobody_read_is_raised_immediately() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        with_draft(&paths, "01A");
        // Mid-morning: too early for a report, but a fresh draft is still worth saying.
        let nudges = due(&paths, &Seen::default(), TODAY, 10, 1, false).expect("due");
        assert_eq!(nudges.len(), 1);
        assert_eq!(nudges[0].reason, Reason::DraftWaiting);
        assert_eq!(nudges[0].route, "/meetings/01A");
    }

    #[test]
    fn the_daily_report_waits_until_the_day_is_over() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        assert!(
            due(&paths, &Seen::default(), TODAY, 14, 2, false)
                .expect("due")
                .is_empty(),
            "nothing to look back on at 2pm"
        );
        let evening = due(&paths, &Seen::default(), TODAY, 18, 2, false).expect("due");
        assert!(evening.iter().any(|n| n.reason == Reason::DailyReport));
    }

    #[test]
    fn a_day_with_no_meetings_gets_no_report() {
        let (_d, paths) = vault(&[("01A", "2026-08-01T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        let nudges = due(&paths, &Seen::default(), TODAY, 20, 2, false).expect("due");
        assert!(
            !nudges.iter().any(|n| n.reason == Reason::DailyReport),
            "{nudges:?}"
        );
    }

    #[test]
    fn overdue_work_is_one_nudge_not_one_per_task() {
        let (_d, paths) = vault(&[(
            "01A",
            "2026-08-01T09:00:00+07:00",
            "## Việc cần làm\n- [ ] một\n- [ ] hai\n- [ ] ba\n- [ ] bốn\n",
        )]);
        let nudges = due(&paths, &Seen::default(), TODAY, 18, 2, false).expect("due");
        let overdue: Vec<_> = nudges
            .iter()
            .filter(|n| n.reason == Reason::Overdue)
            .collect();
        assert_eq!(overdue.len(), 1, "{nudges:?}");
        assert!(overdue[0].title.contains('4'));
    }

    #[test]
    fn the_weekly_rollup_only_lands_on_monday() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        let monday = due(&paths, &Seen::default(), TODAY, 18, 1, false).expect("due");
        assert!(monday.iter().any(|n| n.reason == Reason::WeeklyRollup));

        let tuesday = due(&paths, &Seen::default(), TODAY, 18, 2, false).expect("due");
        assert!(!tuesday.iter().any(|n| n.reason == Reason::WeeklyRollup));
    }

    #[test]
    fn nothing_fires_twice_in_one_day() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        with_draft(&paths, "01A");

        let mut seen = Seen::default();
        let first = due(&paths, &seen, TODAY, 18, 1, false).expect("due");
        assert!(!first.is_empty());
        record(&paths, &mut seen, &first, TODAY).expect("record");

        let second = due(&paths, &seen, TODAY, 18, 1, false).expect("due");
        assert!(second.is_empty(), "said twice: {second:?}");
    }

    #[test]
    fn what_was_said_yesterday_can_be_said_again_today() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        let mut seen = Seen::default();
        let yesterday = due(&paths, &seen, "2026-08-09", 18, 7, false).expect("due");
        record(&paths, &mut seen, &yesterday, "2026-08-09").expect("record");

        let today = due(&paths, &seen, TODAY, 18, 1, false).expect("due");
        assert!(today.iter().any(|n| n.reason == Reason::DailyReport));
    }

    #[test]
    fn what_was_said_survives_a_restart() {
        let (_d, paths) = vault(&[("01A", "2026-08-10T09:00:00+07:00", "## Tóm tắt\nX\n")]);
        let mut seen = Seen::default();
        let nudges = due(&paths, &seen, TODAY, 18, 2, false).expect("due");
        record(&paths, &mut seen, &nudges, TODAY).expect("record");

        // A fresh daemon reads the file rather than starting quiet.
        let reloaded = load(&paths).expect("load");
        assert!(
            due(&paths, &reloaded, TODAY, 18, 2, false)
                .expect("due")
                .is_empty()
        );
    }

    #[test]
    fn old_entries_are_forgotten_so_the_file_does_not_grow_forever() {
        let (_d, _paths) = vault(&[]);
        let mut seen = Seen::default();
        seen.record("daily:2026-01-01", "2026-01-01");
        seen.record("daily:2026-08-09", "2026-08-09");
        seen.prune(TODAY);
        assert!(!seen.already_today("daily:2026-01-01", "2026-01-01"));
        assert_eq!(seen.fired.len(), 1);
    }

    #[test]
    fn a_corrupt_state_file_does_not_stop_the_daemon() {
        let (_d, paths) = vault(&[]);
        std::fs::write(path(&paths), "{ not json").unwrap();
        // Losing "what I already said" costs one duplicate notification. Refusing to start costs
        // the whole feature.
        assert!(load(&paths).is_ok());
    }
}
