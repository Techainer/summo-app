//! A listing of the vault, built by reading files rather than by consulting a database.
//!
//! ADR 0002 measured the alternative: scanning 5,000 meeting files takes ~26 ms, and 1,000 takes
//! ~5 ms, which is below the threshold where a person notices a list appearing. A database would
//! buy nothing except a second source of truth that can disagree with the files.
//!
//! Two things make the scan cheap enough to keep doing it:
//!
//! * Listing reads only the **head** of each file. Frontmatter and the title live in the first few
//!   hundred bytes, so an eight-hour transcript costs the same to list as a one-minute one.
//! * Full text is read only by [`MeetingIndex::search`], and only for the query at hand.
//!
//! The scan is forgiving by design. A file that fails to parse is reported in [`MeetingIndex::skipped`]
//! and the rest of the vault still lists — one malformed document must not blank the library.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use summo_core::{Error, MeetingId, Result};
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::meeting::{Frontmatter, MeetingDoc};

/// How much of a file to read when listing.
///
/// Frontmatter plus a title is a few hundred bytes; 16 KiB is generous enough that a document with
/// an unusually long participant list still parses, and small enough that listing a vault of long
/// meetings does not read hundreds of megabytes.
const HEAD_BYTES: usize = 16 * 1024;

/// Nesting depth allowed under `meetings/`.
///
/// Folders are a user organising their own work, not a tree to recurse without limit; a symlink
/// loop or a stray `node_modules` should not turn a listing into a hang.
const MAX_DEPTH: usize = 8;

/// Section heading whose presence means the meeting has been summarised.
const SUMMARY_HEADINGS: [&str; 2] = ["Tóm tắt", "Summary"];

/// One meeting, as much of it as listing needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingEntry {
    pub id: MeetingId,
    pub path: PathBuf,
    /// Folder under `meetings/`, empty for the root. This is how a user files their own work.
    pub folder: String,
    pub title: String,
    /// The `date` field verbatim, so nothing is lost when it is not a timestamp we can parse.
    pub date: String,
    /// `YYYY-MM-DD` in the meeting's own offset — the day it happened *there*, not here.
    pub day: String,
    /// Seconds since the epoch, or `None` when `date` is not a full timestamp.
    pub started_at: Option<i64>,
    pub duration: u64,
    pub participants: Vec<String>,
    pub tags: Vec<String>,
    /// Whether a summary section exists, so the library can show what still needs one.
    pub has_summary: bool,
    pub size_bytes: u64,
}

impl MeetingEntry {
    /// Sort key: newest first, ties broken by path so the order never flickers between scans.
    ///
    /// A meeting whose `date` could not be parsed sorts last rather than first — an unreadable
    /// date is not evidence of recency, and putting it at the top of the library would be wrong
    /// every time.
    fn ordering_key(&self) -> (i64, &Path) {
        (
            self.started_at.map_or(i64::MAX, |t| -t),
            self.path.as_path(),
        )
    }

    /// ISO week key, `2026-W33`. Weeks are ISO weeks, so a Sunday belongs to the week that started
    /// on Monday rather than to the next one.
    #[must_use]
    pub fn week(&self) -> Option<String> {
        let date = Date::parse(
            &self.day,
            &time::macros::format_description!("[year]-[month]-[day]"),
        )
        .ok()?;
        let (year, week, _) = date.to_iso_week_date();
        Some(format!("{year}-W{week:02}"))
    }
}

/// A group of meetings sharing a day, week or folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group<'a> {
    /// `2026-08-09`, `2026-W33`, or a folder path.
    pub key: String,
    pub meetings: Vec<&'a MeetingEntry>,
}

/// A file that could not be listed, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

/// Totals for the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    pub meetings: usize,
    /// Seconds of recorded audio across the whole vault.
    pub total_duration: u64,
    pub people: usize,
    pub tags: usize,
    pub without_summary: usize,
    /// Meetings in the seven days up to and including `now`.
    pub last_seven_days: usize,
    pub last_seven_days_duration: u64,
    /// Most recent meeting's `date` field, if there is one.
    pub latest: Option<String>,
}

/// A search result: which meeting, and the line that matched.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit<'a> {
    pub meeting: &'a MeetingEntry,
    /// Number of matching lines in the file, which may exceed the excerpts returned.
    pub matches: usize,
    pub excerpts: Vec<Excerpt>,
}

/// One matching line, with where in the recording it was said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Excerpt {
    pub text: String,
    /// Seconds into the recording, when the line is a transcript utterance.
    pub t0: Option<f64>,
    pub speaker: Option<String>,
}

/// The whole listing.
#[derive(Debug, Clone, Default)]
pub struct MeetingIndex {
    entries: Vec<MeetingEntry>,
    skipped: Vec<Skipped>,
}

impl MeetingIndex {
    /// Read every `.md` under `dir`, newest first.
    ///
    /// A missing directory is an empty vault, not an error: a fresh install has not recorded
    /// anything yet, and the library should say "no meetings" rather than fail to open.
    pub fn scan(dir: impl AsRef<Path>) -> Result<Self> {
        let root = dir.as_ref();
        let mut entries = Vec::new();
        let mut skipped = Vec::new();

        for path in markdown_files(root)? {
            match read_entry(&path, root) {
                Ok(entry) => entries.push(entry),
                Err(e) => skipped.push(Skipped {
                    path,
                    reason: e.to_string(),
                }),
            }
        }

        entries.sort_by(|a, b| a.ordering_key().cmp(&b.ordering_key()));
        skipped.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self { entries, skipped })
    }

    #[must_use]
    pub fn entries(&self) -> &[MeetingEntry] {
        &self.entries
    }

    /// Files that failed to parse. Surfacing these is the point: a silently missing meeting is
    /// worse than a visible broken one.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: &MeetingId) -> Option<&MeetingEntry> {
        self.entries.iter().find(|e| &e.id == id)
    }

    /// Group by calendar day, newest day first.
    #[must_use]
    pub fn by_day(&self) -> Vec<Group<'_>> {
        self.group_by(|e| Some(e.day.clone()))
    }

    /// Group by ISO week, newest week first.
    #[must_use]
    pub fn by_week(&self) -> Vec<Group<'_>> {
        self.group_by(MeetingEntry::week)
    }

    /// Group by folder, root first then alphabetically.
    #[must_use]
    pub fn by_folder(&self) -> Vec<Group<'_>> {
        let mut groups = self.group_by(|e| Some(e.folder.clone()));
        groups.sort_by(|a, b| a.key.cmp(&b.key));
        groups
    }

    /// Meetings on one day, in the order they happened.
    #[must_use]
    pub fn on_day(&self, day: &str) -> Vec<&MeetingEntry> {
        let mut on = self.filter(&Filter {
            from: Some(day.to_string()),
            to: Some(day.to_string()),
            ..Default::default()
        });
        on.reverse();
        on
    }

    /// Every folder in use, including the empty root when meetings sit directly under `meetings/`.
    #[must_use]
    pub fn folders(&self) -> Vec<String> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in &self.entries {
            seen.insert(&entry.folder);
            // A meeting in `a/b` means both `a` and `a/b` exist as places to file things.
            let mut prefix = entry.folder.as_str();
            while let Some((parent, _)) = prefix.rsplit_once('/') {
                seen.insert(parent);
                prefix = parent;
            }
        }
        seen.into_iter().map(str::to_string).collect()
    }

    /// Every tag, with how many meetings carry it.
    #[must_use]
    pub fn tags(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for entry in &self.entries {
            for tag in &entry.tags {
                *counts.entry(tag.as_str()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Every participant, with how many meetings they appear in. Wikilink brackets are stripped so
    /// `[[Ngọc]]` and `Ngọc` are the same person.
    #[must_use]
    pub fn people(&self) -> BTreeMap<String, usize> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.entries {
            for person in &entry.participants {
                *counts.entry(unlink(person)).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Meetings matching every filter given. An empty filter matches everything.
    #[must_use]
    pub fn filter(&self, f: &Filter) -> Vec<&MeetingEntry> {
        self.entries.iter().filter(|e| f.matches(e)).collect()
    }

    /// Totals as of `now`.
    ///
    /// The clock is a parameter rather than a call to `OffsetDateTime::now_utc`, so "this week"
    /// is testable and so a caller that already knows the time does not read it twice.
    #[must_use]
    pub fn stats(&self, now: OffsetDateTime) -> Stats {
        let cutoff = now.unix_timestamp() - 7 * 24 * 3600;
        let recent: Vec<&MeetingEntry> = self
            .entries
            .iter()
            .filter(|e| e.started_at.is_some_and(|t| t >= cutoff && t <= now.unix_timestamp()))
            .collect();

        Stats {
            meetings: self.entries.len(),
            total_duration: self.entries.iter().map(|e| e.duration).sum(),
            people: self.people().len(),
            tags: self.tags().len(),
            without_summary: self.entries.iter().filter(|e| !e.has_summary).count(),
            last_seven_days: recent.len(),
            last_seven_days_duration: recent.iter().map(|e| e.duration).sum(),
            latest: self.entries.first().map(|e| e.date.clone()),
        }
    }

    /// Full-text search across the whole document, diacritic-insensitively.
    ///
    /// Typing `hop` finds `họp`. Vietnamese is typed without tone marks often enough — on a phone,
    /// in a hurry, by someone without a Vietnamese keyboard — that a search which demands them is a
    /// search that appears broken. Recall is worth more than precision in a box a person types into.
    ///
    /// `limit` caps the meetings returned, not the files read; results stay newest-first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit<'_>>> {
        let needle = fold(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = Vec::new();
        for entry in &self.entries {
            if hits.len() >= limit {
                break;
            }
            let Ok(text) = std::fs::read_to_string(&entry.path) else {
                // A file readable a moment ago may have been moved; skip it rather than fail the
                // whole search.
                continue;
            };
            let (matches, excerpts) = find_lines(&text, &needle);
            if matches > 0 {
                hits.push(Hit {
                    meeting: entry,
                    matches,
                    excerpts,
                });
            }
        }
        Ok(hits)
    }

    fn group_by<F>(&self, key: F) -> Vec<Group<'_>>
    where
        F: Fn(&MeetingEntry) -> Option<String>,
    {
        // Days and weeks arrive contiguously because entries are sorted by time, but folders do
        // not — checking the last group only would split `khach-hang` into two groups whenever a
        // meeting from elsewhere fell between two of its own.
        let mut groups: Vec<Group<'_>> = Vec::new();
        for entry in &self.entries {
            let Some(key) = key(entry) else { continue };
            match groups.iter_mut().find(|g| g.key == key) {
                Some(group) => group.meetings.push(entry),
                None => groups.push(Group {
                    key,
                    meetings: vec![entry],
                }),
            }
        }
        groups
    }
}

/// Which meetings a caller wants. Every field is an `AND`; within a field, a match is enough.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub person: Option<String>,
    /// Inclusive `YYYY-MM-DD` bounds.
    pub from: Option<String>,
    pub to: Option<String>,
    pub without_summary: bool,
}

impl Filter {
    /// Whether one entry satisfies this filter.
    ///
    /// Public because a caller that groups before it filters — the library screen does — has to
    /// apply the same rules, and a second copy of them would drift.
    #[must_use]
    pub fn matches(&self, e: &MeetingEntry) -> bool {
        if let Some(folder) = &self.folder {
            // A folder filter includes its subfolders: asking for `khach-hang` should not hide
            // `khach-hang/2026`.
            let inside =
                e.folder == *folder || e.folder.starts_with(&format!("{}/", folder.trim_end_matches('/')));
            if !inside {
                return false;
            }
        }
        if let Some(tag) = &self.tag
            && !e.tags.iter().any(|t| fold(t) == fold(tag))
        {
            return false;
        }
        if let Some(person) = &self.person
            && !e.participants.iter().any(|p| fold(&unlink(p)) == fold(person))
        {
            return false;
        }
        if let Some(from) = &self.from
            && e.day.as_str() < from.as_str()
        {
            return false;
        }
        if let Some(to) = &self.to
            && e.day.as_str() > to.as_str()
        {
            return false;
        }
        if self.without_summary && e.has_summary {
            return false;
        }
        true
    }
}

/// Read one file's head and turn it into a listing entry.
fn read_entry(path: &Path, root: &Path) -> Result<MeetingEntry> {
    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let head = read_head(path)?;

    let frontmatter = parse_frontmatter(&head)?;
    let title = parse_title(&head).unwrap_or_else(|| {
        // A file without a heading still deserves a name in the list; the filename is what the
        // user sees in their file manager anyway.
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let (day, started_at) = parse_date(&frontmatter.date);
    let folder = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    Ok(MeetingEntry {
        id: frontmatter.id,
        path: path.to_path_buf(),
        folder,
        title,
        day,
        started_at,
        date: frontmatter.date,
        duration: frontmatter.duration,
        participants: frontmatter.participants,
        tags: frontmatter.tags,
        has_summary: has_summary(&head),
        size_bytes,
    })
}

/// Read at most [`HEAD_BYTES`], stopping at a UTF-8 boundary.
fn read_head(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut buf = vec![0u8; HEAD_BYTES];
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]).map_err(|e| Error::io(path, e))? {
            0 => break,
            n => filled += n,
        }
    }
    buf.truncate(filled);
    // Truncation can land mid-character; drop the partial one rather than fail on valid UTF-8.
    Ok(match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let bytes = e.into_bytes();
            String::from_utf8_lossy(&bytes[..valid]).into_owned()
        }
    })
}

fn parse_frontmatter(head: &str) -> Result<Frontmatter> {
    let rest = head
        .strip_prefix("---\n")
        .ok_or_else(|| Error::Vault("missing YAML frontmatter".into()))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| Error::Vault("frontmatter is not terminated within the file head".into()))?;
    serde_yaml::from_str(&rest[..end])
        .map_err(|e| Error::Vault(format!("cannot parse frontmatter: {e}")))
}

fn parse_title(head: &str) -> Option<String> {
    head.lines()
        .skip_while(|l| !l.starts_with("---"))
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn has_summary(head: &str) -> bool {
    head.lines().any(|line| {
        line.strip_prefix("## ")
            .is_some_and(|h| SUMMARY_HEADINGS.contains(&h.trim()))
    })
}

/// `YYYY-MM-DD` plus, when the field is a full timestamp, the instant it names.
///
/// The day comes from the text rather than from the parsed instant on purpose: a meeting at
/// 23:30 in Hanoi belongs to that day in Hanoi, even when the person reading the list is in Berlin.
fn parse_date(date: &str) -> (String, Option<i64>) {
    let day = date.chars().take(10).collect::<String>();
    let day = if day.len() == 10 && day.as_bytes()[4] == b'-' && day.as_bytes()[7] == b'-' {
        day
    } else {
        String::new()
    };
    let started = OffsetDateTime::parse(date, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp());
    (day, started)
}

/// Walk `root` for `.md` files, bounded in depth and skipping hidden directories.
fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let listing = match std::fs::read_dir(&dir) {
            Ok(l) => l,
            // An unreadable folder is one folder's worth of loss, not the vault's.
            Err(e) => {
                tracing::warn!(path = %dir.display(), error = %e, "skipping unreadable folder");
                continue;
            }
        };
        for entry in listing.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Lines containing `needle`, already folded.
fn find_lines(text: &str, needle: &str) -> (usize, Vec<Excerpt>) {
    /// Excerpts per meeting. Enough to judge relevance, few enough not to flood the list.
    const MAX_EXCERPTS: usize = 3;

    let mut matches = 0;
    let mut excerpts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "---" {
            continue;
        }
        if !fold(line).contains(needle) {
            continue;
        }
        matches += 1;
        if excerpts.len() < MAX_EXCERPTS {
            excerpts.push(excerpt(line));
        }
    }
    (matches, excerpts)
}

/// Turn a matching line into an excerpt, recovering the timestamp when it is a transcript line.
fn excerpt(line: &str) -> Excerpt {
    if let Some(rest) = line.strip_prefix("**[")
        && let Some((timestamp, rest)) = rest.split_once("] ")
        && let Some((speaker, text)) = rest.split_once("** — ")
    {
        return Excerpt {
            text: text.trim().to_string(),
            t0: crate::meeting::parse_timestamp(timestamp),
            speaker: Some(speaker.trim().to_string()),
        };
    }
    Excerpt {
        text: line.to_string(),
        t0: None,
        speaker: None,
    }
}

/// Strip `[[…]]` and any `|alias`.
fn unlink(text: &str) -> String {
    let t = text.trim();
    let inner = t
        .strip_prefix("[[")
        .and_then(|r| r.strip_suffix("]]"))
        .unwrap_or(t);
    inner.split('|').next().unwrap_or(inner).trim().to_string()
}

/// Lowercase and strip Vietnamese tone marks, so `Họp` and `hop` compare equal.
///
/// Shares its table with [`crate::slug`]: a file named `hop-dau-tuan.md` and a search for
/// `hop dau tuan` must agree on what folding means, and two tables would eventually disagree.
#[must_use]
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        out.push(crate::slug::fold_char(ch).unwrap_or(ch));
    }
    out
}

/// Load the full document behind a listing entry.
pub fn load(entry: &MeetingEntry) -> Result<MeetingDoc> {
    let text = std::fs::read_to_string(&entry.path).map_err(|e| Error::io(&entry.path, e))?;
    MeetingDoc::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn meeting(id: &str, date: &str, title: &str) -> String {
        format!(
            "---\nid: {id}\ndate: {date}\nduration: 600\n\
             participants: [\"[[Bạn]]\", \"[[Ngọc]]\"]\ntags: [weekly]\n---\n\
             # {title}\n\n## Tóm tắt\nChốt dùng Rust.\n\n## Transcript\n\
             **[00:12:04] Bạn** — Mình họp về ngân sách nhé\n"
        )
    }

    fn vault() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            root,
            "2026-08-09-weekly-sync.md",
            &meeting("01A", "2026-08-09T10:00:00+07:00", "Weekly Sync"),
        );
        write(
            root,
            "2026-08-09-1-1-ngoc.md",
            &meeting("01B", "2026-08-09T15:00:00+07:00", "1-1 Ngọc"),
        );
        write(
            root,
            "khach-hang/2026-08-05-demo.md",
            &meeting("01C", "2026-08-05T09:00:00+07:00", "Demo khách hàng"),
        );
        dir
    }

    #[test]
    fn an_empty_vault_lists_rather_than_fails() {
        let index = MeetingIndex::scan("/nonexistent/vault/meetings").unwrap();
        assert!(index.is_empty());
        assert!(index.skipped().is_empty());
    }

    #[test]
    fn meetings_list_newest_first() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let titles: Vec<&str> = index.entries().iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["1-1 Ngọc", "Weekly Sync", "Demo khách hàng"]);
    }

    #[test]
    fn a_broken_file_is_reported_and_the_rest_still_list() {
        let dir = vault();
        write(dir.path(), "broken.md", "no frontmatter here\n");
        let index = MeetingIndex::scan(dir.path()).unwrap();

        assert_eq!(index.len(), 3, "the good files must still be listed");
        assert_eq!(index.skipped().len(), 1);
        assert!(index.skipped()[0].path.ends_with("broken.md"));
    }

    #[test]
    fn listing_does_not_read_a_whole_transcript() {
        // A long meeting must cost the same to list as a short one, which is only true if listing
        // stops after the head of the file.
        let dir = TempDir::new().unwrap();
        let mut body = meeting("01A", "2026-08-09T10:00:00+07:00", "Long");
        for i in 0..50_000 {
            body.push_str(&format!("**[00:00:{:02}] Bạn** — dòng {i}\n", i % 60));
        }
        write(dir.path(), "long.md", &body);

        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.entries()[0].title, "Long");
        assert!(
            index.entries()[0].size_bytes > HEAD_BYTES as u64,
            "the fixture must be bigger than the head that was read"
        );
    }

    #[test]
    fn days_group_in_the_meetings_own_timezone() {
        // 23:30 in Hanoi is the previous day in UTC. The list must say the day it happened there.
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "late.md",
            &meeting("01A", "2026-08-09T23:30:00+07:00", "Muộn"),
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.entries()[0].day, "2026-08-09");
    }

    #[test]
    fn grouping_by_day_keeps_days_whole() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let days = index.by_day();

        assert_eq!(days.len(), 2);
        assert_eq!(days[0].key, "2026-08-09");
        assert_eq!(days[0].meetings.len(), 2);
        assert_eq!(days[1].key, "2026-08-05");
    }

    #[test]
    fn grouping_by_week_uses_iso_weeks() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let weeks = index.by_week();
        // 5 Aug 2026 is a Wednesday in W32; 9 Aug is the Sunday that ends W32 — same week.
        assert_eq!(weeks.len(), 1, "got {:?}", weeks.iter().map(|w| &w.key).collect::<Vec<_>>());
        assert_eq!(weeks[0].key, "2026-W32");
        assert_eq!(weeks[0].meetings.len(), 3);
    }

    #[test]
    fn folders_come_from_where_the_user_put_the_file() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.folders(), vec!["", "khach-hang"]);

        let filtered = index.filter(&Filter {
            folder: Some("khach-hang".into()),
            ..Default::default()
        });
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Demo khách hàng");
    }

    #[test]
    fn a_folder_filter_includes_its_subfolders() {
        let dir = vault();
        write(
            dir.path(),
            "khach-hang/acme/2026-08-01-kickoff.md",
            &meeting("01D", "2026-08-01T09:00:00+07:00", "Kickoff"),
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(
            index
                .filter(&Filter {
                    folder: Some("khach-hang".into()),
                    ..Default::default()
                })
                .len(),
            2
        );
        assert!(index.folders().contains(&"khach-hang/acme".to_string()));
    }

    #[test]
    fn people_and_tags_are_counted_across_the_vault() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.people().get("Ngọc"), Some(&3));
        assert_eq!(index.tags().get("weekly"), Some(&3));
    }

    #[test]
    fn stats_count_only_the_last_seven_days_as_recent() {
        let dir = vault();
        // Two months earlier: inside the vault, outside the week.
        write(
            dir.path(),
            "old.md",
            &meeting("01E", "2026-06-01T09:00:00+07:00", "Cũ"),
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let now = OffsetDateTime::parse("2026-08-10T09:00:00+07:00", &Rfc3339).unwrap();
        let stats = index.stats(now);

        assert_eq!(stats.meetings, 4);
        assert_eq!(stats.last_seven_days, 3);
        assert_eq!(stats.last_seven_days_duration, 1800);
        assert_eq!(stats.total_duration, 2400);
        assert_eq!(stats.people, 2);
        assert_eq!(stats.without_summary, 0);
    }

    #[test]
    fn a_meeting_with_no_summary_is_visible_as_such() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "bare.md",
            "---\nid: 01F\ndate: 2026-08-09T10:00:00+07:00\n---\n# Chưa tóm tắt\n",
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert!(!index.entries()[0].has_summary);
        assert_eq!(
            index
                .filter(&Filter {
                    without_summary: true,
                    ..Default::default()
                })
                .len(),
            1
        );
    }

    #[test]
    fn search_finds_a_word_typed_without_tone_marks() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let hits = index.search("hop ve ngan sach", 10).unwrap();
        assert_eq!(hits.len(), 3, "every meeting contains the line");
        assert_eq!(hits[0].excerpts[0].text, "Mình họp về ngân sách nhé");
        assert_eq!(hits[0].excerpts[0].t0, Some(724.0));
        assert_eq!(hits[0].excerpts[0].speaker.as_deref(), Some("Bạn"));
    }

    #[test]
    fn search_with_tone_marks_still_matches() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.search("ngân sách", 10).unwrap().len(), 3);
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_everything() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert!(index.search("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn search_respects_the_limit() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.search("hop", 2).unwrap().len(), 2);
    }

    #[test]
    fn folding_maps_the_vietnamese_alphabet_onto_ascii() {
        assert_eq!(fold("Họp Đầu Tuần"), "hop dau tuan");
        assert_eq!(fold("Nguyễn Thị Bích Ngọc"), "nguyen thi bich ngoc");
        assert_eq!(fold("ASCII stays"), "ascii stays");
    }

    #[test]
    fn filtering_by_person_ignores_wikilink_syntax_and_tone_marks() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(
            index
                .filter(&Filter {
                    person: Some("ngoc".into()),
                    ..Default::default()
                })
                .len(),
            3
        );
    }

    #[test]
    fn a_date_range_filters_inclusively_at_both_ends() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let f = Filter {
            from: Some("2026-08-05".into()),
            to: Some("2026-08-05".into()),
            ..Default::default()
        };
        assert_eq!(index.filter(&f).len(), 1);
    }

    #[test]
    fn hidden_folders_are_not_walked() {
        let dir = vault();
        write(
            dir.path(),
            ".trash/2026-08-09-deleted.md",
            &meeting("01G", "2026-08-09T11:00:00+07:00", "Đã xoá"),
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert_eq!(index.len(), 3, "a file in a dot-folder must stay out of the list");
    }

    #[test]
    fn a_document_can_be_loaded_from_its_listing_entry() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let doc = load(&index.entries()[0]).unwrap();
        assert_eq!(doc.transcript.len(), 1);
        assert_eq!(doc.section("Tóm tắt"), Some("Chốt dùng Rust."));
    }
}
