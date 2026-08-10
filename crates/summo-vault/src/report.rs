//! What a day, or a week, actually contained.
//!
//! This is not a summary — no model runs here. It is arithmetic over what the vault already holds:
//! how long was spent in meetings, with whom, on what, and what was left unfinished. That
//! distinction matters, because it means a report is instant, works offline, costs nothing, and
//! cannot be wrong in the way a generated summary can be wrong.
//!
//! Two things make it worth building rather than leaving to the library screen. First, **open
//! action items** are the only part of a vault that decays: a meeting is a record, but a task
//! nobody looked at again is a failure, and nothing else surfaces them across meetings. Second,
//! **hours in meetings** is a number people consistently underestimate and cannot get anywhere else
//! without adding it up by hand.
//!
//! The day is the meeting's own day — `2026-08-09` in the offset the meeting happened in, not in
//! the reader's timezone. Somebody who flies to Singapore should not see Monday's meetings move to
//! Sunday.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use summo_core::Result;

use crate::index::MeetingIndex;

/// An unfinished task, and the meeting it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub text: String,
    /// Who it was assigned to, from a leading `@name`, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub meeting: String,
    pub meeting_title: String,
    pub day: String,
    pub done: bool,
}

/// Time spent with one person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonTime {
    pub name: String,
    pub meetings: usize,
    pub seconds: u64,
}

/// One meeting, as a report lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportMeeting {
    pub id: String,
    pub title: String,
    pub day: String,
    pub duration: u64,
    pub participants: Vec<String>,
    pub tags: Vec<String>,
    pub has_summary: bool,
}

/// What happened over a span of days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Inclusive `YYYY-MM-DD` bounds. Equal for a single day.
    pub from: String,
    pub to: String,
    pub meetings: Vec<ReportMeeting>,
    pub total_seconds: u64,
    /// Longest first — who the time actually went to.
    pub people: Vec<PersonTime>,
    /// Most frequent first.
    pub tags: Vec<(String, usize)>,
    /// Still open, oldest first: the ones most likely to have been forgotten.
    pub open_actions: Vec<ActionItem>,
    pub done_actions: usize,
    /// Meetings with no summary written yet.
    pub without_summary: Vec<String>,
    /// Days in range that had no meetings at all.
    pub quiet_days: Vec<String>,
}

/// Build a report for `from..=to`, inclusive.
///
/// Both bounds are `YYYY-MM-DD`. A range where `to` precedes `from` yields an empty report rather
/// than an error — it is a query with no answer, not a mistake worth interrupting anyone over.
pub fn between(vault: &std::path::Path, from: &str, to: &str) -> Result<Report> {
    let index = MeetingIndex::scan(vault)?;

    let mut meetings = Vec::new();
    let mut total_seconds = 0u64;
    let mut people: BTreeMap<String, PersonTime> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut without_summary = Vec::new();
    let mut open_actions = Vec::new();
    let mut done_actions = 0usize;
    let mut busy_days: BTreeMap<String, ()> = BTreeMap::new();

    for entry in index.entries() {
        if entry.day.as_str() < from || entry.day.as_str() > to {
            continue;
        }
        busy_days.insert(entry.day.clone(), ());
        total_seconds += entry.duration;

        for name in &entry.participants {
            let slot = people.entry(name.clone()).or_insert_with(|| PersonTime {
                name: name.clone(),
                meetings: 0,
                seconds: 0,
            });
            slot.meetings += 1;
            slot.seconds += entry.duration;
        }
        for tag in &entry.tags {
            *tags.entry(tag.clone()).or_default() += 1;
        }
        if !entry.has_summary {
            without_summary.push(entry.title.clone());
        }

        // Actions need the body, which the head-only scan deliberately does not read. A report is
        // not on the listing path, so paying for a full read here is the right trade.
        let path = vault.join(&entry.path);
        if let Ok(body) = std::fs::read_to_string(&path) {
            // One parser, in `tasks.rs`. A second copy here drifted from it the moment the task
            // format grew ids, statuses and agent steps.
            for task in crate::tasks::parse(&body, &entry.path.display().to_string()) {
                if task.status.is_finished() {
                    done_actions += 1;
                    continue;
                }
                open_actions.push(ActionItem {
                    text: task.text,
                    owner: task.owner,
                    meeting: entry.id.to_string(),
                    meeting_title: entry.title.clone(),
                    day: entry.day.clone(),
                    done: false,
                });
            }
        }

        meetings.push(ReportMeeting {
            id: entry.id.to_string(),
            title: entry.title.clone(),
            day: entry.day.clone(),
            duration: entry.duration,
            participants: entry.participants.clone(),
            tags: entry.tags.clone(),
            has_summary: entry.has_summary,
        });
    }

    let mut people: Vec<PersonTime> = people.into_values().collect();
    // Longest first, name as the tiebreak so the order is stable between runs.
    people.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));

    let mut tags: Vec<(String, usize)> = tags.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // Oldest first: an action from three weeks ago is more urgent than one from this morning.
    open_actions.sort_by(|a, b| a.day.cmp(&b.day).then_with(|| a.text.cmp(&b.text)));

    Ok(Report {
        quiet_days: days_between(from, to)
            .into_iter()
            .filter(|d| !busy_days.contains_key(d))
            .collect(),
        from: from.to_string(),
        to: to.to_string(),
        meetings,
        total_seconds,
        people,
        tags,
        open_actions,
        done_actions,
        without_summary,
    })
}

/// One day.
pub fn day(vault: &std::path::Path, day: &str) -> Result<Report> {
    between(vault, day, day)
}

/// Every `YYYY-MM-DD` from `from` to `to`, inclusive.
///
/// Capped so a nonsense range cannot allocate for a decade. Two years is more than any report a
/// person reads.
fn days_between(from: &str, to: &str) -> Vec<String> {
    const MAX_DAYS: usize = 750;
    let Some(mut current) = parse(from) else {
        return Vec::new();
    };
    let Some(end) = parse(to) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    while current <= end && out.len() < MAX_DAYS {
        out.push(format(current));
        current = next_day(current);
    }
    out
}

/// A date as `(year, month, day)`, which is all the arithmetic here needs.
fn parse(day: &str) -> Option<(i32, u32, u32)> {
    let mut parts = day.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let date = parts.next()?.parse().ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&date).then_some(())?;
    Some((year, month, date))
}

fn format((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn next_day((y, m, d): (i32, u32, u32)) -> (i32, u32, u32) {
    if d < days_in_month(y, m) {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let meetings = dir.path().join("meetings");
        std::fs::create_dir_all(&meetings).expect("mkdir");
        for (name, body) in files {
            std::fs::write(meetings.join(name), body).expect("write");
        }
        dir
    }

    fn meeting(id: &str, date: &str, title: &str, extra: &str) -> String {
        format!(
            "---\nid: {id}\ndate: {date}\nduration: 1800\n\
             participants: [\"Bạn\", \"Ngọc\"]\ntags: [weekly]\n---\n# {title}\n{extra}\n"
        )
    }

    #[test]
    fn an_empty_vault_reports_a_quiet_day_rather_than_failing() {
        let dir = TempDir::new().unwrap();
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert!(report.meetings.is_empty());
        assert_eq!(report.total_seconds, 0);
        assert_eq!(report.quiet_days, vec!["2026-08-10"]);
    }

    #[test]
    fn a_day_counts_only_its_own_meetings() {
        let dir = vault_with(&[
            ("a.md", &meeting("01A", "2026-08-10T09:00:00+07:00", "Sáng", "")),
            ("b.md", &meeting("01B", "2026-08-10T15:00:00+07:00", "Chiều", "")),
            ("c.md", &meeting("01C", "2026-08-11T09:00:00+07:00", "Mai", "")),
        ]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.meetings.len(), 2);
        assert_eq!(report.total_seconds, 3600);
        assert!(report.quiet_days.is_empty());
    }

    /// The meeting's own offset decides its day, not the reader's timezone.
    #[test]
    fn a_late_meeting_in_a_positive_offset_stays_on_its_own_day() {
        let dir = vault_with(&[(
            "a.md",
            &meeting("01A", "2026-08-10T23:30:00+07:00", "Muộn", ""),
        )]);
        assert_eq!(day(dir.path(), "2026-08-10").unwrap().meetings.len(), 1);
        assert_eq!(day(dir.path(), "2026-08-10").unwrap().quiet_days.len(), 0);
    }

    #[test]
    fn time_is_attributed_to_everyone_who_was_there() {
        let dir = vault_with(&[(
            "a.md",
            &meeting("01A", "2026-08-10T09:00:00+07:00", "Họp", ""),
        )]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.people.len(), 2);
        assert_eq!(report.people[0].seconds, 1800);
        assert_eq!(report.people[0].meetings, 1);
    }

    #[test]
    fn people_are_ordered_by_time_not_by_name() {
        let long = "---\nid: 01A\ndate: 2026-08-10T09:00:00+07:00\nduration: 7200\n\
             participants: [\"Zung\"]\ntags: []\n---\n# Dài\n";
        let short = "---\nid: 01B\ndate: 2026-08-10T12:00:00+07:00\nduration: 600\n\
             participants: [\"An\"]\ntags: []\n---\n# Ngắn\n";
        let dir = vault_with(&[("a.md", long), ("b.md", short)]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.people[0].name, "Zung", "two hours outranks ten minutes");
    }

    #[test]
    fn open_action_items_are_collected_across_meetings() {
        let dir = vault_with(&[
            (
                "a.md",
                &meeting(
                    "01A",
                    "2026-08-10T09:00:00+07:00",
                    "Họp",
                    "## Action items\n- [ ] @Ngọc chốt spec API\n- [x] Gửi biên bản\n",
                ),
            ),
            (
                "b.md",
                &meeting(
                    "01B",
                    "2026-08-10T14:00:00+07:00",
                    "Họp 2",
                    "## Việc cần làm\n- [ ] Đặt lịch demo\n",
                ),
            ),
        ]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.open_actions.len(), 2);
        assert_eq!(report.done_actions, 1);
        assert_eq!(report.open_actions[0].owner.as_deref(), Some("Ngọc"));
        assert!(report.open_actions[1].owner.is_none());
    }

    /// A transcript line that happens to look like a checkbox is not a task.
    #[test]
    fn checkboxes_outside_an_action_heading_are_ignored() {
        let dir = vault_with(&[(
            "a.md",
            &meeting(
                "01A",
                "2026-08-10T09:00:00+07:00",
                "Họp",
                "## Transcript\n- [ ] không phải việc\n",
            ),
        )]);
        assert!(day(dir.path(), "2026-08-10").unwrap().open_actions.is_empty());
    }

    /// A heading after the action list closes it.
    #[test]
    fn an_action_list_ends_at_the_next_heading() {
        let dir = vault_with(&[(
            "a.md",
            &meeting(
                "01A",
                "2026-08-10T09:00:00+07:00",
                "Họp",
                "## Action items\n- [ ] thật\n## Transcript\n- [ ] giả\n",
            ),
        )]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.open_actions.len(), 1);
        assert_eq!(report.open_actions[0].text, "thật");
    }

    #[test]
    fn the_oldest_open_action_comes_first() {
        let dir = vault_with(&[
            (
                "a.md",
                &meeting(
                    "01A",
                    "2026-08-03T09:00:00+07:00",
                    "Cũ",
                    "## Action items\n- [ ] việc cũ\n",
                ),
            ),
            (
                "b.md",
                &meeting(
                    "01B",
                    "2026-08-10T09:00:00+07:00",
                    "Mới",
                    "## Action items\n- [ ] việc mới\n",
                ),
            ),
        ]);
        let report = between(dir.path(), "2026-08-01", "2026-08-14").expect("report");
        assert_eq!(report.open_actions[0].text, "việc cũ");
    }

    #[test]
    fn a_week_reports_the_days_that_had_nothing() {
        let dir = vault_with(&[(
            "a.md",
            &meeting("01A", "2026-08-10T09:00:00+07:00", "Thứ hai", ""),
        )]);
        let report = between(dir.path(), "2026-08-10", "2026-08-12").expect("report");
        assert_eq!(report.quiet_days, vec!["2026-08-11", "2026-08-12"]);
    }

    #[test]
    fn meetings_without_a_summary_are_named() {
        let dir = vault_with(&[(
            "a.md",
            &meeting("01A", "2026-08-10T09:00:00+07:00", "Chưa tóm tắt", ""),
        )]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.without_summary, vec!["Chưa tóm tắt"]);
    }

    #[test]
    fn a_backwards_range_is_empty_rather_than_an_error() {
        let dir = vault_with(&[(
            "a.md",
            &meeting("01A", "2026-08-10T09:00:00+07:00", "Họp", ""),
        )]);
        let report = between(dir.path(), "2026-08-12", "2026-08-10").expect("report");
        assert!(report.meetings.is_empty());
        assert!(report.quiet_days.is_empty());
    }

    #[test]
    fn days_between_crosses_a_month_boundary() {
        assert_eq!(
            days_between("2026-01-30", "2026-02-02"),
            vec!["2026-01-30", "2026-01-31", "2026-02-01", "2026-02-02"]
        );
    }

    #[test]
    fn days_between_crosses_a_year_boundary() {
        assert_eq!(
            days_between("2025-12-31", "2026-01-01"),
            vec!["2025-12-31", "2026-01-01"]
        );
    }

    #[test]
    fn february_has_a_twenty_ninth_in_a_leap_year() {
        assert_eq!(
            days_between("2028-02-28", "2028-03-01"),
            vec!["2028-02-28", "2028-02-29", "2028-03-01"]
        );
        // 2100 is divisible by 4 but not a leap year.
        assert_eq!(
            days_between("2100-02-28", "2100-03-01"),
            vec!["2100-02-28", "2100-03-01"]
        );
    }

    #[test]
    fn a_nonsense_range_cannot_allocate_forever() {
        assert!(days_between("2000-01-01", "2999-01-01").len() <= 750);
    }

    #[test]
    fn an_unparseable_day_yields_nothing_rather_than_panicking() {
        assert!(days_between("hôm nay", "2026-01-01").is_empty());
        assert!(days_between("2026-13-01", "2026-14-01").is_empty());
    }

    /// Owner parsing itself is tested in `tasks.rs`; this pins that the report still surfaces it,
    /// which is the part a caller depends on.
    #[test]
    fn an_owner_survives_into_the_report() {
        let dir = vault_with(&[(
            "a.md",
            &meeting(
                "01A",
                "2026-08-10T09:00:00+07:00",
                "Họp",
                "## Việc cần làm\n- [ ] @Ngọc chốt spec\n- [ ] hỏi @Ngọc về spec\n",
            ),
        )]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.open_actions.len(), 2);
        let owners: Vec<Option<&str>> =
            report.open_actions.iter().map(|a| a.owner.as_deref()).collect();
        assert!(owners.contains(&Some("Ngọc")), "{owners:?}");
        assert!(owners.contains(&None), "a mid-line mention is not an owner: {owners:?}");
    }

    /// The report and the board must agree about what is finished.
    #[test]
    fn a_task_marked_done_by_status_counts_as_done() {
        let dir = vault_with(&[(
            "a.md",
            &meeting(
                "01A",
                "2026-08-10T09:00:00+07:00",
                "Họp",
                "## Việc cần làm\n- [ ] xong rồi <!-- id:1 status:done -->\n- [ ] chưa\n",
            ),
        )]);
        let report = day(dir.path(), "2026-08-10").expect("report");
        assert_eq!(report.done_actions, 1);
        assert_eq!(report.open_actions.len(), 1);
        assert_eq!(report.open_actions[0].text, "chưa");
    }
}
