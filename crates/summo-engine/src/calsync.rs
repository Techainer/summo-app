//! Subscribing to a calendar, so the agenda keeps up on its own.
//!
//! [`crate::agenda`] reads whatever `.ics` files are in `~/.summo/calendars/`, and until now the
//! only way one got there was [`crate::agenda::install`], which copies a file the user picked. That
//! is a fine way to look at a calendar once and a bad way to use one: an export is a snapshot, so
//! the agenda quietly describes last Tuesday until somebody exports again.
//!
//! Every calendar people actually use publishes a URL that is always current:
//!
//! | | Where the URL comes from |
//! |---|---|
//! | Google Calendar | Settings for the calendar → *Secret address in iCal format* |
//! | Apple Calendar / iCloud | Right-click the calendar → *Share* → *Public Calendar* |
//! | Outlook / Microsoft 365 | Settings → *Shared calendars* → publish → ICS link |
//! | Anything else | Almost every calendar server serves iCalendar over HTTP |
//!
//! So a subscription is a URL plus a name, and syncing is fetching it and overwriting the file.
//! That is deliberately the whole design — **no OAuth**. OAuth would mean registering Summo with
//! Google, shipping a client secret in an open-source binary that anybody can read, and asking for
//! an account-wide scope so an offline notes app can learn what time the standup is. A secret URL
//! the user chooses to paste grants exactly one calendar, is revocable from the calendar's own
//! settings, and needs no server of ours to exist.
//!
//! **This still never starts a recording.** It fills the agenda and, with
//! `Recording::suggest_on_meeting`, lets [`crate::nudge`] ask whether to take notes. Asking is the
//! strongest thing a calendar is allowed to cause.
//!
//! The URL is a secret in its own right — anyone holding it can read the calendar — which is why it
//! lives in `~/.summo/calendars/subscriptions.json` beside the data it fetches, and why it is
//! deliberately outside the vault: a synced vault would carry an employer's calendar key to every
//! other machine.

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};

/// A calendar the app fetches for itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// File stem in `calendars/`, and the id in every route. Slugged, so it is safe as a path.
    pub name: String,
    /// What the user called it, which may be Vietnamese and is never used as a filename.
    pub title: String,
    pub url: String,
    /// Seconds since the epoch of the last successful fetch, or `None` if it has never worked.
    #[serde(default)]
    pub last_sync: Option<i64>,
    /// Why the last attempt failed, kept until one succeeds.
    ///
    /// Shown rather than logged: a subscription that stopped working looks exactly like a week with
    /// no meetings, and the difference matters.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Events in the file as of the last fetch.
    #[serde(default)]
    pub events: usize,
}

/// A calendar body larger than this is not a calendar.
///
/// A year of a busy calendar is a few hundred kilobytes. The cap is here because the response comes
/// from a URL a user pasted, and a redirect to something enormous should cost a moment rather than
/// the machine's memory.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How long to wait for a calendar server.
const TIMEOUT_S: u64 = 20;

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.calendars().join("subscriptions.json")
}

/// Every subscription, in the order they were added.
pub fn list(paths: &Paths) -> Result<Vec<Subscription>> {
    match std::fs::read_to_string(file(paths)) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::io(file(paths), e)),
    }
}

fn store(paths: &Paths, subscriptions: &[Subscription]) -> Result<()> {
    let dir = paths.calendars();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    summo_vault::write::write_atomically(
        &file(paths),
        serde_json::to_vec_pretty(subscriptions)?.as_slice(),
    )
}

/// Turn what a user pasted into a URL that can be fetched, or refuse it.
///
/// `webcal://` is what Apple and Outlook hand out and what every browser refuses; it is plain HTTPS
/// with a scheme that means "subscribe to this". Rewriting it here is the difference between the
/// Apple flow working and the user having to know that.
///
/// Everything that is not HTTP is refused rather than attempted. `file:///etc/passwd` is a URL, the
/// daemon can read it, and "the app fetches whatever string it is handed" is how a local-first app
/// becomes a file-exfiltration tool.
pub fn normalise(url: &str) -> Result<String> {
    let url = url.trim();
    let rewritten = match url.split_once("://") {
        Some(("webcal" | "webcals", rest)) => format!("https://{rest}"),
        Some(("http" | "https", _)) => url.to_string(),
        _ => {
            return Err(Error::msg(
                "calendar.scheme",
                "địa chỉ lịch phải bắt đầu bằng https:// hoặc webcal://",
            ));
        }
    };
    if rewritten
        .split_once("://")
        .is_none_or(|(_, rest)| rest.is_empty() || rest.starts_with('/'))
    {
        return Err(Error::msg("calendar.url", "địa chỉ lịch không hợp lệ"));
    }
    Ok(rewritten)
}

/// Fetch a calendar and write it into `calendars/`, returning how many events it holds.
///
/// The file is replaced only when the response parses as a calendar with events in it. Calendar
/// servers answer expired links with a login page carrying `200 OK`, and writing that over a
/// working calendar would empty the agenda for a reason nobody could see.
async fn fetch(paths: &Paths, name: &str, url: &str) -> Result<usize> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_S))
        .build()
        .map_err(|e| Error::msg("calendar.client", e.to_string()))?;

    let response = client
        .get(url)
        .header("Accept", "text/calendar, text/plain;q=0.5")
        .send()
        .await
        .map_err(|e| Error::msg("calendar.fetch", friendly(&e)))?;

    let status = response.status();
    if !status.is_success() {
        return Err(Error::msg(
            "calendar.http",
            match status.as_u16() {
                401 | 403 => "lịch từ chối truy cập — kiểm tra lại địa chỉ bí mật".to_string(),
                404 => "không tìm thấy lịch ở địa chỉ này".to_string(),
                other => format!("máy chủ lịch trả về {other}"),
            },
        ));
    }
    if response.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err(Error::msg("calendar.too_big", "tệp lịch quá lớn"));
    }

    let text = response
        .text()
        .await
        .map_err(|e| Error::msg("calendar.body", friendly(&e)))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(Error::msg("calendar.too_big", "tệp lịch quá lớn"));
    }

    let events = summo_calendar::ics::parse(&text);
    if events.is_empty() {
        return Err(Error::msg(
            "calendar.no_events",
            if text.contains("BEGIN:VCALENDAR") {
                "lịch này chưa có sự kiện nào"
            } else {
                "địa chỉ này không trả về lịch"
            },
        ));
    }

    let dir = paths.calendars();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    summo_vault::write::write_atomically(&dir.join(format!("{name}.ics")), text.as_bytes())?;
    Ok(events.len())
}

/// reqwest's own `Display` is a chain of internal types; this is what a person needs to read.
fn friendly(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "máy chủ lịch không trả lời".to_string()
    } else if e.is_connect() {
        "không kết nối được tới máy chủ lịch".to_string()
    } else {
        e.to_string()
    }
}

/// Add a subscription and fetch it once, so a URL that does not work fails while the user is
/// looking at the form rather than silently at three in the morning.
pub async fn subscribe(paths: &Paths, title: &str, url: &str) -> Result<Subscription> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::msg("calendar.no_name", "lịch cần có tên"));
    }
    let url = normalise(url)?;

    let mut subscriptions = list(paths)?;
    if let Some(existing) = subscriptions.iter().find(|s| s.url == url) {
        return Err(Error::msg(
            "calendar.duplicate",
            format!("lịch này đã có rồi: {}", existing.title),
        ));
    }
    let name = unique_name(title, &subscriptions);

    // Fetch before recording it. A subscription saved first and fetched second leaves a broken row
    // in the list when the URL is wrong, and the user has to delete it before trying again.
    let events = fetch(paths, &name, &url).await?;

    let subscription = Subscription {
        name,
        title: title.to_string(),
        url,
        last_sync: Some(now()),
        last_error: None,
        events,
    };
    subscriptions.push(subscription.clone());
    store(paths, &subscriptions)?;
    Ok(subscription)
}

/// Fetch every subscription, or one of them.
///
/// One failure does not stop the others, and does not remove anything: a laptop that woke up
/// without WiFi should still show this morning's meetings from the file it already has.
pub async fn refresh(paths: &Paths, only: Option<&str>) -> Result<Vec<Subscription>> {
    let mut subscriptions = list(paths)?;
    for subscription in &mut subscriptions {
        if only.is_some_and(|name| name != subscription.name) {
            continue;
        }
        match fetch(paths, &subscription.name, &subscription.url).await {
            Ok(events) => {
                subscription.events = events;
                subscription.last_sync = Some(now());
                subscription.last_error = None;
            }
            Err(e) => {
                tracing::warn!(calendar = %subscription.name, error = %e, "calendar refresh failed");
                subscription.last_error = Some(e.to_string());
            }
        }
    }
    store(paths, &subscriptions)?;
    Ok(subscriptions)
}

/// Forget a subscription and the file it fetched.
///
/// Both, because leaving the `.ics` behind means a calendar the user removed keeps appearing in the
/// agenda with no row anywhere to remove it a second time.
pub fn unsubscribe(paths: &Paths, name: &str) -> Result<bool> {
    let mut subscriptions = list(paths)?;
    let before = subscriptions.len();
    subscriptions.retain(|s| s.name != name);
    let removed = subscriptions.len() != before;
    if removed {
        store(paths, &subscriptions)?;
        crate::agenda::forget(paths, name)?;
    }
    Ok(removed)
}

/// A file stem for a title, unique among the ones already taken.
///
/// [`crate::agenda::safe_name`] keeps ASCII only, so "Lịch công ty" becomes empty and a Vietnamese
/// user could not name a calendar in Vietnamese. Slugging handles the accents, and the title is
/// kept separately for display, so the file is `lich-cong-ty.ics` and the interface still says
/// "Lịch công ty".
fn unique_name(title: &str, existing: &[Subscription]) -> String {
    let base = {
        let slug = summo_vault::slug::slugify(title);
        if slug.is_empty() {
            "calendar".to_string()
        } else {
            slug
        }
    };
    let taken = |name: &str| existing.iter().any(|s| s.name == name);
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken(name))
        .unwrap_or(base)
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// How often a running daemon re-fetches, in seconds.
///
/// Fifteen minutes. A calendar changes on the timescale of somebody moving a meeting, and the point
/// of syncing at all is that the change is there before the meeting is. Hourly would miss the
/// common case of a meeting moved that morning; every minute would be a request every minute,
/// forever, to somebody else's server.
pub const REFRESH_S: u64 = 900;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_and_outlook_links_are_rewritten_rather_than_refused() {
        assert_eq!(
            normalise("webcal://p1-caldav.icloud.com/published/2/abc").unwrap(),
            "https://p1-caldav.icloud.com/published/2/abc"
        );
        assert_eq!(
            normalise("  https://calendar.google.com/basic.ics  ").unwrap(),
            "https://calendar.google.com/basic.ics"
        );
    }

    /// The daemon fetches this URL with the user's file permissions. A scheme that reads the disk
    /// or talks to a local socket is not a calendar, whatever it is called.
    #[test]
    fn only_http_is_fetched() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/cal.ics",
            "/home/me/cal.ics",
            "javascript:alert(1)",
            "https://",
            "https:///etc/passwd",
        ] {
            assert!(normalise(url).is_err(), "{url} should be refused");
        }
    }

    #[test]
    fn a_vietnamese_name_still_produces_a_usable_filename() {
        let name = unique_name("Lịch công ty", &[]);
        assert!(!name.is_empty());
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{name} must be safe as a path"
        );
    }

    #[test]
    fn two_calendars_with_the_same_name_do_not_share_a_file() {
        let first = Subscription {
            name: unique_name("Việc", &[]),
            title: "Việc".into(),
            url: "https://a".into(),
            last_sync: None,
            last_error: None,
            events: 0,
        };
        let second = unique_name("Việc", std::slice::from_ref(&first));
        assert_ne!(first.name, second);
    }

    #[test]
    fn listing_an_empty_installation_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        assert!(list(&paths).unwrap().is_empty());
        assert!(!unsubscribe(&paths, "nothing").unwrap());
    }
}
