//! Calendars, and matching a recording to the meeting it was for.
//!
//! Two jobs. Reading iCalendar — the format a `.ics` export, a subscription URL and a CalDAV
//! response all speak, which is why [`ics`] is one parser rather than three. And deciding which
//! calendar event a recording belongs to, which is the part with an opinion in it.
//!
//! **Nothing here records anything.** Meeting auto-detection stays opt-in and never starts a
//! recording on its own; `Recording::suggest_on_meeting` in `summo_core::settings` is the switch,
//! and the strongest thing this crate does is name the event a recording probably belongs to. An
//! app that starts listening because a calendar said so is an app that records a therapy
//! appointment somebody put in their work calendar.

pub mod ics;

pub use ics::{Event, When};

/// How confident a match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// The recording started inside the event's window.
    Inside,
    /// It started near the event — people join late and start recording later.
    Near,
}

/// A recording matched to an event.
#[derive(Debug, Clone, PartialEq)]
pub struct Match<'a> {
    pub event: &'a Event,
    pub confidence: Confidence,
    /// Seconds between the recording starting and the event starting. Negative means early.
    pub offset_s: i64,
}

/// How far outside an event a recording may start and still be its recording.
///
/// Ten minutes each way. People join a call a few minutes early and remember to press record a few
/// minutes late, and a window narrower than that misses the common case; a window much wider starts
/// claiming the meeting before or after.
pub const NEAR_S: i64 = 600;

/// Find the event a recording most likely belongs to.
///
/// `started_epoch` is when recording began. Events are ranked by how close they are, and an event
/// the recording started *inside* always beats one it merely started near — so a back-to-back pair
/// resolves to the one actually in progress rather than the one about to begin.
///
/// Returns `None` rather than guessing when nothing is close. A wrong title on a meeting note is
/// worse than no title: the user has to notice it is wrong before they can fix it.
#[must_use]
pub fn best_match(events: &[Event], started_epoch: i64) -> Option<Match<'_>> {
    let mut best: Option<Match<'_>> = None;

    for event in events {
        let Some(start) = event.start.as_ref() else {
            continue;
        };
        // An all-day block is not a meeting anyone records, and it would swallow every recording
        // made that day.
        if start.all_day() {
            continue;
        }

        let start_epoch = start.approx_epoch();
        let end_epoch = event
            .end
            .as_ref()
            .map_or(start_epoch + 3_600, When::approx_epoch);

        let offset_s = started_epoch - start_epoch;
        let confidence = if started_epoch >= start_epoch && started_epoch <= end_epoch {
            Confidence::Inside
        } else if offset_s.abs() <= NEAR_S {
            Confidence::Near
        } else {
            continue;
        };

        let candidate = Match {
            event,
            confidence,
            offset_s,
        };
        let better = match &best {
            None => true,
            // `Inside` sorts before `Near`, so a smaller confidence wins; ties break on distance.
            Some(current) => (candidate.confidence, candidate.offset_s.abs())
                < (current.confidence, current.offset_s.abs()),
        };
        if better {
            best = Some(candidate);
        }
    }

    best
}

/// Events worth offering to record, in time order.
///
/// Filtered to things that look like meetings — see [`Event::looks_like_a_meeting`] — because a
/// list that includes focus blocks and birthdays is a list nobody reads.
#[must_use]
pub fn meetings(events: &[Event]) -> Vec<&Event> {
    let mut out: Vec<&Event> = events
        .iter()
        .filter(|e| e.looks_like_a_meeting())
        .collect();
    out.sort_by_key(|e| e.start.as_ref().map_or(i64::MAX, When::approx_epoch));
    out
}

/// Read a `.ics` file from disk.
pub fn read(path: &std::path::Path) -> summo_core::Result<Vec<Event>> {
    let text = std::fs::read_to_string(path).map_err(|e| summo_core::Error::io(path, e))?;
    Ok(ics::parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hh: u8, mm: u8) -> When {
        When::Utc {
            y: 2026,
            m: 8,
            d: 10,
            hh,
            mm,
            ss: 0,
        }
    }

    fn event(uid: &str, start: When, end: Option<When>) -> Event {
        Event {
            uid: uid.into(),
            summary: uid.into(),
            start: Some(start),
            end,
            location: None,
            description: None,
            attendees: vec!["a@x".into(), "b@x".into()],
            organizer: None,
            conference: None,
            rrule: None,
        }
    }

    #[test]
    fn a_recording_inside_an_event_matches_it() {
        let events = [event("standup", at(9, 0), Some(at(9, 30)))];
        let m = best_match(&events, at(9, 10).approx_epoch()).unwrap();
        assert_eq!(m.event.uid, "standup");
        assert_eq!(m.confidence, Confidence::Inside);
        assert_eq!(m.offset_s, 600);
    }

    /// People press record a few minutes after joining, and a window narrower than that misses the
    /// common case.
    #[test]
    fn a_recording_started_a_few_minutes_early_still_matches() {
        let events = [event("standup", at(9, 0), Some(at(9, 30)))];
        let m = best_match(&events, at(8, 55).approx_epoch()).unwrap();
        assert_eq!(m.confidence, Confidence::Near);
        assert_eq!(m.offset_s, -300);
    }

    /// The case that decides the rule: two meetings back to back, and the recording is running in
    /// the first while the second is minutes away.
    #[test]
    fn an_event_in_progress_beats_one_about_to_start() {
        let events = [
            event("now", at(9, 0), Some(at(10, 0))),
            event("next", at(10, 0), Some(at(11, 0))),
        ];
        let m = best_match(&events, at(9, 55).approx_epoch()).unwrap();
        assert_eq!(m.event.uid, "now");
        assert_eq!(m.confidence, Confidence::Inside);
    }

    #[test]
    fn the_nearest_of_two_equally_confident_events_wins() {
        let events = [
            event("far", at(8, 40), Some(at(8, 45))),
            event("close", at(8, 58), Some(at(8, 59))),
        ];
        let m = best_match(&events, at(9, 0).approx_epoch()).unwrap();
        assert_eq!(m.event.uid, "close");
    }

    /// A wrong title on a meeting note is worse than no title: the user has to notice it is wrong
    /// before they can fix it.
    #[test]
    fn nothing_close_matches_nothing() {
        let events = [event("morning", at(9, 0), Some(at(9, 30)))];
        assert!(best_match(&events, at(15, 0).approx_epoch()).is_none());
    }

    /// An all-day block would otherwise swallow every recording made that day.
    #[test]
    fn an_all_day_block_never_claims_a_recording() {
        let events = [event(
            "holiday",
            When::Date {
                y: 2026,
                m: 8,
                d: 10,
            },
            None,
        )];
        assert!(best_match(&events, at(9, 0).approx_epoch()).is_none());
    }

    #[test]
    fn an_event_with_no_end_is_assumed_to_run_an_hour() {
        let events = [event("open", at(9, 0), None)];
        assert_eq!(
            best_match(&events, at(9, 45).approx_epoch())
                .unwrap()
                .confidence,
            Confidence::Inside
        );
    }

    #[test]
    fn an_empty_calendar_matches_nothing_without_panicking() {
        assert!(best_match(&[], at(9, 0).approx_epoch()).is_none());
    }

    #[test]
    fn meetings_are_listed_in_time_order() {
        let events = [
            event("later", at(14, 0), Some(at(15, 0))),
            event("earlier", at(9, 0), Some(at(10, 0))),
        ];
        let listed: Vec<_> = meetings(&events).iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(listed, ["earlier", "later"]);
    }

    #[test]
    fn a_focus_block_is_not_listed_as_a_meeting() {
        let mut solo = event("focus", at(9, 0), Some(at(10, 0)));
        solo.attendees.clear();
        assert!(meetings(&[solo]).is_empty());
    }

    #[test]
    fn reading_a_file_from_disk_returns_its_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cal.ics");
        std::fs::write(
            &path,
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260810T090000Z\r\nSUMMARY:Họp\r\nEND:VEVENT\r\n",
        )
        .unwrap();

        let events = read(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Họp");
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let err = read(std::path::Path::new("/nonexistent/cal.ics"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("cal.ics"), "{err}");
    }
}
