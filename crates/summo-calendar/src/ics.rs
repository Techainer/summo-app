//! Reading iCalendar, which is older and stranger than it looks.
//!
//! Every calendar Summo can reach speaks this: a `.ics` export from Google, a subscription URL from
//! Fastmail, the body of a CalDAV response. So one parser serves all three, and CalDAV becomes a
//! transport question rather than a format one.
//!
//! Four details in RFC 5545 that a naive line-splitting parser gets wrong, and which this handles
//! because real calendars rely on all of them:
//!
//! * **Folding.** Lines longer than 75 octets are wrapped, and a continuation begins with a space
//!   or a tab. Unfolding has to happen before anything else, or a long summary arrives cut in half
//!   with a stray space in the middle.
//! * **Parameters.** `DTSTART;TZID=Asia/Ho_Chi_Minh:20260810T090000` — the property name ends at
//!   the first `;`, not at the `:`, and splitting on `:` alone puts the timezone in the name.
//! * **Escaping.** `\,` `\;` `\n` `\\` inside text values. A meeting called "Q4: budget, scope" is
//!   stored escaped, and printing it raw shows the backslashes.
//! * **All-day events.** `VALUE=DATE` with no time. Treating those as midnight-local is what makes
//!   a birthday show up as a meeting at 00:00.
//!
//! Timezones are read but not resolved. `TZID` names an Olson zone and resolving it needs a
//! database this crate deliberately does not carry; what it does instead is keep the name alongside
//! the naive time so a caller with a database can finish the job, and treat `Z` as the UTC it
//! actually is. Guessing would put meetings on the wrong day for anyone east of Greenwich, which is
//! everyone this is built for.

use std::collections::BTreeMap;

/// One property line, after unfolding.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    /// Parameters after the name, e.g. `TZID` or `VALUE`.
    pub params: BTreeMap<String, String>,
    pub value: String,
}

/// When something happens, as the file says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// A date with no time: a birthday, a holiday, an all-day block.
    Date { y: i32, m: u8, d: u8 },
    /// A local time, with the zone named if the file named one.
    Local {
        y: i32,
        m: u8,
        d: u8,
        hh: u8,
        mm: u8,
        ss: u8,
        /// Olson zone from `TZID`, unresolved on purpose.
        tzid: Option<String>,
    },
    /// An instant in UTC — the `Z` suffix.
    Utc {
        y: i32,
        m: u8,
        d: u8,
        hh: u8,
        mm: u8,
        ss: u8,
    },
}

impl When {
    /// The calendar day this falls on, as `YYYY-MM-DD`.
    ///
    /// For a local or floating time this is the day *where the event is*, which is the day a person
    /// would name it by. For a UTC instant it is the UTC day, and a caller that knows the user's
    /// zone should convert first — this cannot, and pretending otherwise would file a 23:00 meeting
    /// under tomorrow for anyone in Hanoi.
    #[must_use]
    pub fn day(&self) -> String {
        match self {
            When::Date { y, m, d } | When::Local { y, m, d, .. } | When::Utc { y, m, d, .. } => {
                format!("{y:04}-{m:02}-{d:02}")
            }
        }
    }

    #[must_use]
    pub fn all_day(&self) -> bool {
        matches!(self, When::Date { .. })
    }

    /// Seconds since the Unix epoch, for ordering and for comparing against a recording.
    ///
    /// A local time with an unresolved `TZID` is treated as UTC for this purpose *and only this
    /// purpose* — ordering two events in the same calendar is still correct, because they share the
    /// same offset error. Anything that needs a real instant has to resolve `tzid` first.
    #[must_use]
    pub fn approx_epoch(&self) -> i64 {
        let (y, m, d, hh, mm, ss) = match *self {
            When::Date { y, m, d } => (y, m, d, 0, 0, 0),
            When::Local {
                y,
                m,
                d,
                hh,
                mm,
                ss,
                ..
            }
            | When::Utc {
                y,
                m,
                d,
                hh,
                mm,
                ss,
            } => (y, m, d, hh, mm, ss),
        };
        days_from_civil(y, m, d) * 86_400
            + i64::from(hh) * 3_600
            + i64::from(mm) * 60
            + i64::from(ss)
    }
}

/// Days from 1970-01-01 to a civil date. Howard Hinnant's algorithm, which is correct for the whole
/// proleptic Gregorian range rather than only for dates near now.
fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(m) + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One `VEVENT`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// `UID`, stable across edits — what links a recording to the meeting it was for.
    pub uid: String,
    pub summary: String,
    pub start: Option<When>,
    pub end: Option<When>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// `ATTENDEE` addresses, without the `mailto:` prefix.
    pub attendees: Vec<String>,
    /// `ORGANIZER`, likewise.
    pub organizer: Option<String>,
    /// A conferencing link, from `X-GOOGLE-CONFERENCE`, `CONFERENCE` or the first URL in the body.
    pub conference: Option<String>,
    /// Present when the event repeats. Not expanded — see the note on [`parse`].
    pub rrule: Option<String>,
}

impl Event {
    /// Whether this looks like a meeting rather than a block of time.
    ///
    /// A calendar is full of things that are not meetings: focus blocks, birthdays, travel. Two or
    /// more attendees, or a conferencing link, is the cheapest signal that other people are
    /// involved — and offering to record a birthday is how an app teaches someone to ignore it.
    #[must_use]
    pub fn looks_like_a_meeting(&self) -> bool {
        if self.start.as_ref().is_some_and(When::all_day) {
            return false;
        }
        self.conference.is_some() || self.attendees.len() >= 2
    }

    /// How long it runs, in seconds, when both ends are known.
    #[must_use]
    pub fn duration_s(&self) -> Option<i64> {
        let start = self.start.as_ref()?.approx_epoch();
        let end = self.end.as_ref()?.approx_epoch();
        (end >= start).then_some(end - start)
    }
}

/// Unfold, then split into property lines.
///
/// RFC 5545 folds at 75 octets and continues with a leading space or tab. Doing this first, on the
/// whole document, is the difference between a parser that works on hand-written test data and one
/// that works on a real export.
#[must_use]
pub fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        out.push(line.to_string());
    }
    out
}

/// Split one unfolded line into name, parameters and value.
///
/// Returns `None` for a line with no `:` at all, which is a blank or a broken line rather than a
/// property. A parameter value may itself be quoted and contain a colon — `DTSTART;TZID="X:Y":…` —
/// so the scan tracks quoting rather than taking the first colon.
#[must_use]
pub fn property(line: &str) -> Option<Property> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = line.split_at(colon);
    let value = &value[1..];

    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }

    let mut params = BTreeMap::new();
    for part in parts {
        if let Some((k, v)) = part.split_once('=') {
            params.insert(
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            );
        }
    }

    Some(Property {
        name,
        params,
        value: value.to_string(),
    })
}

/// Undo the text escaping in RFC 5545 §3.3.11.
///
/// Without this, "Q4: budget, scope" comes back as `Q4: budget\, scope`, which is what the user
/// sees on screen.
#[must_use]
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(',') => out.push(','),
            Some(';') => out.push(';'),
            Some('\\') => out.push('\\'),
            // An unknown escape keeps both characters: dropping them would silently eat text.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Parse a date-time value, using the parameters to decide what kind it is.
#[must_use]
pub fn when(property: &Property) -> Option<When> {
    let value = property.value.trim();
    let is_date = property.params.get("VALUE").map(String::as_str) == Some("DATE");

    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    let y: i32 = digits.get(0..4)?.parse().ok()?;
    let m: u8 = digits.get(4..6)?.parse().ok()?;
    let d: u8 = digits.get(6..8)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }

    if is_date || digits.len() < 14 {
        return Some(When::Date { y, m, d });
    }

    let hh: u8 = digits.get(8..10)?.parse().ok()?;
    let mm: u8 = digits.get(10..12)?.parse().ok()?;
    let ss: u8 = digits.get(12..14)?.parse().ok()?;
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }

    if value.ends_with('Z') || value.ends_with('z') {
        Some(When::Utc {
            y,
            m,
            d,
            hh,
            mm,
            ss,
        })
    } else {
        Some(When::Local {
            y,
            m,
            d,
            hh,
            mm,
            ss,
            tzid: property.params.get("TZID").cloned(),
        })
    }
}

/// Every `VEVENT` in a calendar document.
///
/// **Recurrence is not expanded.** `RRULE` is kept verbatim so a caller can see that an event
/// repeats, but generating the occurrences correctly means handling `EXDATE`, `RECURRENCE-ID`
/// overrides and DST transitions, and a half-correct expansion silently puts meetings on the wrong
/// days. Better to carry the rule and say so than to invent dates.
#[must_use]
pub fn parse(text: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Event> = None;

    for line in unfold(text) {
        let Some(prop) = property(&line) else {
            continue;
        };

        match (prop.name.as_str(), prop.value.trim()) {
            ("BEGIN", "VEVENT") => {
                current = Some(Event {
                    uid: String::new(),
                    summary: String::new(),
                    start: None,
                    end: None,
                    location: None,
                    description: None,
                    attendees: Vec::new(),
                    organizer: None,
                    conference: None,
                    rrule: None,
                });
                continue;
            }
            ("END", "VEVENT") => {
                if let Some(event) = current.take() {
                    // An event with no `UID` cannot be matched to anything later, and calendars do
                    // not produce one — a file that does is truncated or hand-written.
                    if !event.uid.is_empty() {
                        events.push(event);
                    }
                }
                continue;
            }
            _ => {}
        }

        let Some(event) = current.as_mut() else {
            continue;
        };

        match prop.name.as_str() {
            "UID" => event.uid = prop.value.trim().to_string(),
            "SUMMARY" => event.summary = unescape(prop.value.trim()),
            "DTSTART" => event.start = when(&prop),
            "DTEND" => event.end = when(&prop),
            "LOCATION" => event.location = non_empty(unescape(prop.value.trim())),
            "DESCRIPTION" => event.description = non_empty(unescape(prop.value.trim())),
            "RRULE" => event.rrule = non_empty(prop.value.trim().to_string()),
            "ORGANIZER" => event.organizer = non_empty(address(&prop.value)),
            "ATTENDEE" => {
                if let Some(who) = non_empty(address(&prop.value)) {
                    event.attendees.push(who);
                }
            }
            "X-GOOGLE-CONFERENCE" | "CONFERENCE" => {
                event.conference = non_empty(prop.value.trim().to_string());
            }
            _ => {}
        }
    }

    // A conferencing link is often only in the description, which is where Zoom and Meet put it.
    for event in &mut events {
        if event.conference.is_none() {
            event.conference = event.description.as_deref().and_then(first_meeting_url);
        }
    }
    events
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// Strip `mailto:` and any display name, leaving the address.
fn address(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("MAILTO:")
        .trim_start_matches("mailto:")
        .trim()
        .to_string()
}

/// The first conferencing URL in a block of text.
///
/// Restricted to hosts that are actually conferencing services. Taking any URL would pick up the
/// agenda document somebody linked, and offering to join that is worse than offering nothing.
#[must_use]
pub fn first_meeting_url(text: &str) -> Option<String> {
    const HOSTS: [&str; 6] = [
        "meet.google.com",
        "zoom.us",
        "teams.microsoft.com",
        "teams.live.com",
        "whereby.com",
        "meet.jit.si",
    ];

    text.split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_graphic() || "<>(),;".contains(c)))
        .find(|word| {
            (word.starts_with("https://") || word.starts_with("http://"))
                && HOSTS.iter().any(|host| word.contains(host))
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOGLE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:abc123@google.com\r\n\
DTSTART;TZID=Asia/Ho_Chi_Minh:20260810T090000\r\n\
DTEND;TZID=Asia/Ho_Chi_Minh:20260810T100000\r\n\
SUMMARY:Họp ngân sách Q4\r\n\
LOCATION:Phòng 3\r\n\
ATTENDEE;CN=Ngoc:mailto:ngoc@example.com\r\n\
ATTENDEE;CN=Binh:mailto:binh@example.com\r\n\
ORGANIZER:mailto:ngoc@example.com\r\n\
X-GOOGLE-CONFERENCE:https://meet.google.com/abc-defg-hij\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn a_google_export_parses() {
        let events = parse(GOOGLE);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.uid, "abc123@google.com");
        assert_eq!(e.summary, "Họp ngân sách Q4");
        assert_eq!(e.location.as_deref(), Some("Phòng 3"));
        assert_eq!(e.attendees, ["ngoc@example.com", "binh@example.com"]);
        assert_eq!(e.organizer.as_deref(), Some("ngoc@example.com"));
        assert_eq!(
            e.conference.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    /// The property name ends at the first `;`, not the first `:`. Splitting on `:` alone puts
    /// `TZID=Asia/Ho_Chi_Minh` in the name and loses the start time entirely.
    #[test]
    fn a_parameter_does_not_become_part_of_the_name() {
        let p = property("DTSTART;TZID=Asia/Ho_Chi_Minh:20260810T090000").unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.params["TZID"], "Asia/Ho_Chi_Minh");
        assert_eq!(p.value, "20260810T090000");
    }

    #[test]
    fn a_quoted_parameter_may_contain_a_colon() {
        let p = property(r#"DTSTART;TZID="GMT+07:00":20260810T090000"#).unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.params["TZID"], "GMT+07:00");
        assert_eq!(p.value, "20260810T090000");
    }

    /// Real exports fold at 75 octets. Without unfolding, a long title arrives cut in half.
    #[test]
    fn a_folded_line_is_rejoined_before_anything_else() {
        let folded = "SUMMARY:Họp về ngân sách quý bốn và\r\n  kế hoạch tuyển dụng";
        let lines = unfold(folded);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("kế hoạch tuyển dụng"), "{}", lines[0]);
    }

    #[test]
    fn a_tab_continuation_folds_too() {
        assert_eq!(unfold("A:one\r\n\ttwo").len(), 1);
    }

    /// Printing the raw value shows the backslashes to the user.
    #[test]
    fn escaped_text_is_unescaped() {
        assert_eq!(
            unescape(r"Q4: budget\, scope\; done"),
            "Q4: budget, scope; done"
        );
        assert_eq!(unescape(r"line\nline"), "line\nline");
        assert_eq!(unescape(r"back\\slash"), r"back\slash");
    }

    /// Dropping an unrecognised escape would silently eat text.
    #[test]
    fn an_unknown_escape_keeps_both_characters() {
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape("trailing\\"), "trailing\\");
    }

    #[test]
    fn a_utc_instant_is_recognised_by_its_z() {
        let p = property("DTSTART:20260810T020000Z").unwrap();
        assert_eq!(
            when(&p),
            Some(When::Utc {
                y: 2026,
                m: 8,
                d: 10,
                hh: 2,
                mm: 0,
                ss: 0
            })
        );
    }

    /// Treating an all-day event as midnight is what makes a birthday show up as a 00:00 meeting.
    #[test]
    fn an_all_day_event_is_a_date_not_a_midnight() {
        let p = property("DTSTART;VALUE=DATE:20260810").unwrap();
        let w = when(&p).unwrap();
        assert!(w.all_day());
        assert_eq!(w.day(), "2026-08-10");
    }

    #[test]
    fn a_local_time_keeps_the_zone_name_rather_than_guessing_at_it() {
        let p = property("DTSTART;TZID=Asia/Ho_Chi_Minh:20260810T090000").unwrap();
        let When::Local { tzid, hh, .. } = when(&p).unwrap() else {
            panic!("expected a local time")
        };
        assert_eq!(tzid.as_deref(), Some("Asia/Ho_Chi_Minh"));
        assert_eq!(hh, 9);
    }

    #[test]
    fn a_nonsense_date_is_refused_rather_than_wrapped_around() {
        assert_eq!(when(&property("DTSTART:20261301T090000").unwrap()), None);
        assert_eq!(when(&property("DTSTART:20260810T250000").unwrap()), None);
        assert_eq!(when(&property("DTSTART:nope").unwrap()), None);
    }

    #[test]
    fn a_leap_second_is_accepted_because_calendars_emit_them() {
        assert!(when(&property("DTSTART:20261231T235960Z").unwrap()).is_some());
    }

    /// An event with no UID cannot be matched to a recording later, and no real calendar emits one.
    #[test]
    fn an_event_without_a_uid_is_dropped() {
        let text = "BEGIN:VEVENT\r\nSUMMARY:no id\r\nEND:VEVENT\r\n";
        assert!(parse(text).is_empty());
    }

    #[test]
    fn several_events_in_one_file_all_come_back() {
        let text = format!("{GOOGLE}{GOOGLE}");
        assert_eq!(parse(&text).len(), 2);
    }

    /// Zoom and Meet both put the link in the description rather than a property.
    #[test]
    fn a_link_in_the_description_is_found() {
        let text = "BEGIN:VEVENT\r\nUID:x\r\n\
DESCRIPTION:Join here: https://zoom.us/j/123456 or dial in\r\nEND:VEVENT\r\n";
        assert_eq!(
            parse(text)[0].conference.as_deref(),
            Some("https://zoom.us/j/123456")
        );
    }

    /// Taking any URL would pick up the agenda document somebody linked, and offering to join that
    /// is worse than offering nothing.
    #[test]
    fn an_ordinary_link_is_not_mistaken_for_a_conference() {
        assert_eq!(
            first_meeting_url("Agenda at https://docs.example.com/plan"),
            None
        );
    }

    #[test]
    fn a_repeating_event_carries_its_rule_rather_than_being_expanded() {
        let text = "BEGIN:VEVENT\r\nUID:x\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEND:VEVENT\r\n";
        assert_eq!(
            parse(text)[0].rrule.as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO")
        );
    }

    #[test]
    fn duration_comes_from_both_ends_or_not_at_all() {
        let e = &parse(GOOGLE)[0];
        assert_eq!(e.duration_s(), Some(3_600));

        let mut open = e.clone();
        open.end = None;
        assert_eq!(open.duration_s(), None);
    }

    /// An end before the start is a broken file, not a negative meeting.
    #[test]
    fn an_end_before_the_start_reports_no_duration() {
        let mut e = parse(GOOGLE)[0].clone();
        e.end = Some(When::Local {
            y: 2026,
            m: 8,
            d: 10,
            hh: 8,
            mm: 0,
            ss: 0,
            tzid: None,
        });
        assert_eq!(e.duration_s(), None);
    }

    /// A calendar is full of things that are not meetings. Offering to record a birthday is how an
    /// app teaches somebody to ignore it.
    #[test]
    fn a_meeting_is_told_apart_from_a_block_of_time() {
        assert!(parse(GOOGLE)[0].looks_like_a_meeting());

        let solo = "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260810T090000Z\r\n\
SUMMARY:Focus time\r\nEND:VEVENT\r\n";
        assert!(!parse(solo)[0].looks_like_a_meeting());

        let birthday = "BEGIN:VEVENT\r\nUID:y\r\nDTSTART;VALUE=DATE:20260810\r\n\
SUMMARY:Sinh nhật Ngọc\r\nATTENDEE:mailto:a@x\r\nATTENDEE:mailto:b@x\r\nEND:VEVENT\r\n";
        assert!(
            !parse(birthday)[0].looks_like_a_meeting(),
            "all-day is not a meeting"
        );
    }

    #[test]
    fn one_attendee_alone_is_not_a_meeting() {
        let text = "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260810T090000Z\r\n\
ATTENDEE:mailto:me@example.com\r\nEND:VEVENT\r\n";
        assert!(!parse(text)[0].looks_like_a_meeting());
    }

    #[test]
    fn epochs_order_events_correctly_across_a_month_boundary() {
        let jan31 = When::Utc {
            y: 2026,
            m: 1,
            d: 31,
            hh: 23,
            mm: 0,
            ss: 0,
        };
        let feb01 = When::Utc {
            y: 2026,
            m: 2,
            d: 1,
            hh: 0,
            mm: 0,
            ss: 0,
        };
        assert!(jan31.approx_epoch() < feb01.approx_epoch());
        assert_eq!(feb01.approx_epoch() - jan31.approx_epoch(), 3_600);
    }

    #[test]
    fn the_epoch_of_the_epoch_is_zero() {
        assert_eq!(
            When::Utc {
                y: 1970,
                m: 1,
                d: 1,
                hh: 0,
                mm: 0,
                ss: 0
            }
            .approx_epoch(),
            0
        );
    }

    #[test]
    fn a_leap_year_is_handled() {
        let feb29 = When::Date {
            y: 2024,
            m: 2,
            d: 29,
        };
        let mar01 = When::Date {
            y: 2024,
            m: 3,
            d: 1,
        };
        assert_eq!(mar01.approx_epoch() - feb29.approx_epoch(), 86_400);
    }

    #[test]
    fn an_empty_document_parses_to_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").is_empty());
    }

    #[test]
    fn a_line_with_no_colon_is_skipped_rather_than_crashing() {
        assert!(property("garbage").is_none());
        assert!(property("").is_none());
        assert!(property(":no name").is_none());
    }
}
