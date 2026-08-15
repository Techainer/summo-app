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

use serde::{Deserialize, Serialize};
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
    /// Whether anything was said out loud.
    ///
    /// A meeting and a typed note are the same document — see `crate::note`. This is the one thing
    /// that distinguishes them, and it is derived from the transcript rather than from the folder,
    /// so moving a file does not change what it is.
    #[serde(default)]
    pub kind: Kind,
    pub path: PathBuf,
    /// Folder under `meetings/`, empty for the root. This is how a user files their own work.
    pub folder: String,
    /// The page this one lives inside, when it is a sub-page.
    ///
    /// A folder and a parent are two different structures over the same set and both are the
    /// user's: a folder is where the file *is*, a parent is what the page is *part of*. A sub-page
    /// keeps whichever folder it was filed in, because moving a child out of its parent's directory
    /// is a filing decision and detaching it from its parent is not.
    ///
    /// A document naming itself as its parent is dropped here rather than at the point of drawing —
    /// see [`read_entry`]. Every reader of this field would otherwise need the same guard, and the
    /// one that forgot would loop.
    pub parent: Option<MeetingId>,
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
    /// A palette name, already validated — never the raw string from the file.
    ///
    /// Listing is where a colour crosses from a file somebody edits into something a screen paints,
    /// so it is where the raw value stops. See [`crate::colour`].
    pub color: Option<&'static str>,
    /// Whether a summary section exists, so the library can show what still needs one.
    pub has_summary: bool,
    pub size_bytes: u64,
}

/// What a document is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Recorded: it has a transcript.
    #[default]
    Meeting,
    /// Typed: it does not.
    Note,
}

impl Kind {
    #[must_use]
    pub fn is_note(self) -> bool {
        matches!(self, Kind::Note)
    }
}

impl MeetingEntry {
    /// Sort key: newest first, ties broken by path so the order never flickers between scans.
    ///
    /// A meeting whose `date` could not be parsed sorts last rather than first — an unreadable
    /// date is not evidence of recency, and putting it at the top of the library would be wrong
    /// every time.
    /// Newest first when reversed. Exported because notes sort the same way meetings do, and two
    /// orderings that disagree would list the same file differently on two screens.
    #[must_use]
    pub fn ordering_key(&self) -> (i64, &Path) {
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
        Self::scan_all([dir.as_ref()])
    }

    /// Read several trees into one index.
    ///
    /// The vault keeps recordings and typed notes in separate folders, because the library is
    /// organised by when a meeting happened and a note typed on a Tuesday has no such day. They are
    /// still the same documents, and every question a user asks — search, tasks, "what did we
    /// decide" — is a question about both. One index over both is what makes that true without a
    /// second scanner that would eventually disagree with the first.
    ///
    /// A folder that is not there is skipped, not an error: a vault with no notes yet is normal.
    pub fn scan_all<'a>(dirs: impl IntoIterator<Item = &'a Path>) -> Result<Self> {
        let mut entries = Vec::new();
        let mut skipped = Vec::new();

        for root in dirs {
            Self::scan_into(root, &mut entries, &mut skipped)?;
        }

        entries.sort_by(|a, b| a.ordering_key().cmp(&b.ordering_key()));
        skipped.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self { entries, skipped })
    }

    fn scan_into(
        root: &Path,
        entries: &mut Vec<MeetingEntry>,
        skipped: &mut Vec<Skipped>,
    ) -> Result<()> {
        for path in markdown_files(root)? {
            match read_entry(&path, root) {
                Ok(entry) => entries.push(entry),
                Err(e) => skipped.push(Skipped {
                    path,
                    reason: e.to_string(),
                }),
            }
        }
        Ok(())
    }

    /// Only the recordings.
    pub fn meetings(&self) -> impl Iterator<Item = &MeetingEntry> {
        self.entries.iter().filter(|e| !e.kind.is_note())
    }

    /// Only what somebody typed.
    pub fn notes(&self) -> impl Iterator<Item = &MeetingEntry> {
        self.entries.iter().filter(|e| e.kind.is_note())
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

    /// Every colour in use, with how many documents carry it.
    ///
    /// Only colours actually on something: an eight-swatch picker belongs on one note's own
    /// controls, whereas the finder should offer the four a user has really filed by. Offering all
    /// eight there would be five dead filters and one live one.
    #[must_use]
    pub fn colours(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for colour in self.entries.iter().filter_map(|e| e.color) {
            *counts.entry(colour).or_insert(0) += 1;
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
            .filter(|e| {
                e.started_at
                    .is_some_and(|t| t >= cutoff && t <= now.unix_timestamp())
            })
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
    /// Only entries of this kind.
    ///
    /// A meeting is a recording with a transcript; a note is typed. Both live in the same vault and
    /// the library has always returned both — what was missing was any way to ask for one. A
    /// workspace whose second navigation item lists only meetings is a workspace telling the user
    /// meetings are the product, which they are not.
    pub kind: Option<Kind>,
    pub folder: Option<String>,
    /// Every tag named must be present, not merely one of them.
    ///
    /// A list rather than a single tag because narrowing is the point of a finder: `khách-hàng` on
    /// its own is a hundred notes, and `khách-hàng` + `hợp-đồng` is the four somebody is looking
    /// for. An empty list matches everything.
    pub tags: Vec<String>,
    pub colour: Option<String>,
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
        if let Some(kind) = self.kind
            && e.kind != kind
        {
            return false;
        }
        if let Some(folder) = &self.folder {
            // A folder filter includes its subfolders: asking for `khach-hang` should not hide
            // `khach-hang/2026`.
            let inside = e.folder == *folder
                || e.folder
                    .starts_with(&format!("{}/", folder.trim_end_matches('/')));
            if !inside {
                return false;
            }
        }
        // Every tag asked for, not any: see the field's own note.
        for tag in &self.tags {
            if !e.tags.iter().any(|t| fold(t) == fold(tag)) {
                return false;
            }
        }
        // Compared against the *normalised* colour, so filtering by `green` finds the note whose
        // file says `#0f7350` — the same colour, spelled the way its author spells colours.
        //
        // A filter naming no colour we know matches nothing. Letting it fall through to
        // `e.color == None` would have answered "which notes are green?" with every uncoloured
        // note in the vault, which is the most confidently wrong answer available.
        if let Some(colour) = &self.colour {
            match crate::colour::normalise(colour) {
                Some(wanted) if e.color == Some(wanted) => {}
                _ => return false,
            }
        }
        if let Some(person) = &self.person
            && !e
                .participants
                .iter()
                .any(|p| fold(&unlink(p)) == fold(person))
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
    let meta = std::fs::metadata(path).ok();
    let size_bytes = meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
    let head = read_head(path)?;

    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut frontmatter = match parse_frontmatter(&head) {
        Ok(frontmatter) => frontmatter,
        // No frontmatter means a person wrote this file, not Summo. That is the ordinary way a
        // note gets into a folder somebody was told they own, so it is adopted rather than
        // reported: an id from its path, a date from when it was last written.
        Err(_) if !head.starts_with("---\n") => {
            Frontmatter::new(MeetingId::from(String::new()), String::new())
        }
        Err(e) => return Err(e),
    };
    // The same two fallbacks whether the file had no frontmatter or partial frontmatter, which is
    // what Obsidian writes when somebody adds a tag: an id from the path, so it survives a rescan,
    // and a date from the file's own mtime, so a note written last March stays in March.
    frontmatter.fill(adopted_id(relative), modified_at(meta.as_ref()));

    let title = parse_title(&head).unwrap_or_else(|| {
        // A file without a heading still deserves a name in the list; the filename is what the
        // user sees in their file manager anyway.
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let color = frontmatter.swatch();
    let (day, started_at) = parse_date(&frontmatter.date);
    let folder = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    // A file that names itself as its own parent has no parent. Hand-edited frontmatter is the
    // likely source, and the cost of trusting it is a row that is its own ancestor — which is not a
    // wrong drawing but an infinite one.
    let parent = frontmatter
        .parent
        .filter(|p| !p.as_str().is_empty() && p != &frontmatter.id);

    Ok(MeetingEntry {
        id: frontmatter.id,
        kind: kind_of(&head, frontmatter.duration),
        path: path.to_path_buf(),
        folder,
        parent,
        title,
        day,
        started_at,
        date: frontmatter.date,
        duration: frontmatter.duration,
        participants: frontmatter.participants,
        tags: frontmatter.tags,
        color,
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
        match file
            .read(&mut buf[filled..])
            .map_err(|e| Error::io(path, e))?
        {
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
    // Skipping to the `---` avoids mistaking a `#` inside frontmatter for the title — but only when
    // there *is* frontmatter. On an adopted file the skip consumed the whole document and every
    // hand-written note listed under its filename instead of its heading.
    let lines = head.lines();
    let body: Box<dyn Iterator<Item = &str>> = if head.starts_with("---\n") {
        Box::new(lines.skip_while(|l| !l.starts_with("---")))
    } else {
        Box::new(lines)
    };
    body.filter_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())
}

/// A stable id for a file Summo did not write.
///
/// Derived from the path, not generated, because the id has to survive a rescan: a fresh ULID each
/// time would give the same note a new identity every launch, breaking every link, comment and task
/// that pointed at it. Prefixed so an adopted document is recognisable in a log or a filename.
#[must_use]
pub fn adopted_id(relative: &Path) -> MeetingId {
    let key = relative.to_string_lossy().replace('\\', "/");
    let digest = blake3::hash(key.as_bytes()).to_hex();
    MeetingId::from(format!("adopted-{}", &digest[..24]))
}

/// When a file was last written, as the date a document with no frontmatter is filed under.
///
/// Its own modification time, not today: a note written last March belongs in March, and filing it
/// under the day the vault happened to be scanned would reorder somebody's whole library.
fn modified_at(meta: Option<&std::fs::Metadata>) -> String {
    meta.and_then(|m| m.modified().ok())
        .map(OffsetDateTime::from)
        .and_then(|t| {
            // The local offset is what the rest of the vault records, and it is unavailable in a
            // process with threads on some platforms — UTC is a correct answer, not a wrong one.
            t.to_offset(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
                .format(&Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Whether a document is a recording or something somebody typed.
///
/// Listing reads only the head of a file, so this cannot count transcript segments the way
/// `crate::note::is_note` does. Two signals instead, and either is enough:
///
/// * A `## Transcript` heading in the head. Present in every recording, and impossible in a note —
///   a note has no transcript to head.
/// * A non-zero `duration`. This is the safety net for the one case the first signal misses: a
///   meeting whose summary is long enough to push its transcript heading past the head window.
///   Anything recorded has a duration; nothing typed does.
///
/// Derived from the document rather than from the folder on purpose. Filing by path would mean a
/// user dragging a file between folders silently changed what it is.
fn kind_of(head: &str, duration: u64) -> Kind {
    let has_transcript = head.lines().any(|line| {
        line.strip_prefix("## ")
            .is_some_and(|h| h.trim() == "Transcript")
    });

    if has_transcript || duration > 0 {
        Kind::Meeting
    } else {
        Kind::Note
    }
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

/// A person's name, without the wikilink syntax around it.
///
/// `[[Ngọc]]` is how a name is stored so Obsidian links it; it is not how anybody is called. Every
/// screen that shows a participant has to strip it, and the copies drift — the analytics screen was
/// listing `[[Bạn]]`, `[[Ngọc]]` and `[[Bình]]` in a bar chart because the report path was the one
/// place that had never grown its own copy.
#[must_use]
pub fn unlink(text: &str) -> String {
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
///
/// Adopting rather than parsing, and with the id and date the listing already worked out: an
/// adopted file that listed fine but would not *open* is the same bug one screen later.
pub fn load(entry: &MeetingEntry) -> Result<MeetingDoc> {
    let text = std::fs::read_to_string(&entry.path).map_err(|e| Error::io(&entry.path, e))?;
    MeetingDoc::adopt(&text, entry.id.clone(), &entry.date)
}

/// Read a document from a path, adopting it if a person wrote it.
///
/// The one way anything should open a vault file. Adoption first landed in the listing and in
/// [`load`], and every *other* reader — `summo summarize`, `summo export`, `summo dub`, the
/// daemon's summariser — went on calling [`MeetingDoc::parse`] and went on refusing the same files.
/// One fix behind four doors, three of which were still shut.
///
/// `vault` is the root the id is derived relative to, so a file has the same identity here as it
/// does in the listing. Getting that wrong would mean `summo summarize` writing an id into a file
/// that the library then disagrees with.
pub fn open(vault: &Path, path: &Path) -> Result<MeetingDoc> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let relative = path.strip_prefix(vault).unwrap_or(path);
    let meta = std::fs::metadata(path).ok();
    MeetingDoc::adopt(&text, adopted_id(relative), modified_at(meta.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note and a meeting are the same document; the transcript is what tells them apart, and
    /// listing has to work that out from the head of the file alone.
    #[test]
    fn a_document_with_a_transcript_is_a_meeting() {
        assert_eq!(
            kind_of("# Họp\n\n## Transcript\n**[00:00:00] ?** — hi", 0),
            Kind::Meeting
        );
    }

    #[test]
    fn a_document_with_neither_is_a_note() {
        assert_eq!(kind_of("# Ý tưởng\n\nvài dòng", 0), Kind::Note);
        assert!(kind_of("# Ý tưởng", 0).is_note());
    }

    /// The case the first signal misses: a summary long enough to push the transcript heading past
    /// the head window. Anything recorded has a duration; nothing typed does.
    #[test]
    fn a_long_meeting_whose_transcript_heading_is_beyond_the_head_is_still_a_meeting() {
        assert_eq!(
            kind_of("# Họp\n\n## Tóm tắt\nrất dài…", 1800),
            Kind::Meeting
        );
    }

    /// Filing by path would mean dragging a file between folders silently changed what it is.
    #[test]
    fn the_kind_does_not_depend_on_where_the_file_lives() {
        let typed = "# Ghi chú\n\nnội dung";
        assert_eq!(kind_of(typed, 0), Kind::Note, "wherever it is filed");
    }
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

    /// Broken means frontmatter that *claims* to be frontmatter and is not. A file with none at all
    /// is not broken — see the adoption tests below.
    #[test]
    fn a_broken_file_is_reported_and_the_rest_still_list() {
        let dir = vault();
        write(dir.path(), "broken.md", "---\nid: [unterminated\n");
        let index = MeetingIndex::scan(dir.path()).unwrap();

        assert_eq!(index.len(), 3, "the good files must still be listed");
        assert_eq!(index.skipped().len(), 1);
        assert!(index.skipped()[0].path.ends_with("broken.md"));
    }

    /// The ordinary way a note arrives: somebody typed it in Obsidian. Rejecting it made the
    /// vault's whole promise false, and did it loudly — one error banner per file.
    #[test]
    fn a_file_a_person_wrote_by_hand_is_adopted_rather_than_skipped() {
        let dir = vault();
        write(dir.path(), "idea.md", "# Ý tưởng\n\nvài dòng\n");
        let index = MeetingIndex::scan(dir.path()).unwrap();

        assert!(index.skipped().is_empty(), "{:?}", index.skipped());
        let adopted = index
            .entries()
            .iter()
            .find(|e| e.path.ends_with("idea.md"))
            .expect("the hand-written file must list");
        assert_eq!(
            adopted.title, "Ý tưởng",
            "its own heading, not its filename"
        );
        assert!(adopted.kind.is_note());
    }

    /// An id derived from the path rather than generated. A fresh one per scan would give the same
    /// note a new identity every launch, breaking every link and task that pointed at it.
    #[test]
    fn an_adopted_id_is_the_same_on_every_scan() {
        let dir = vault();
        write(dir.path(), "idea.md", "# Ý tưởng\n");
        let first = MeetingIndex::scan(dir.path()).unwrap();
        let again = MeetingIndex::scan(dir.path()).unwrap();

        let id_of = |index: &MeetingIndex| {
            index
                .entries()
                .iter()
                .find(|e| e.path.ends_with("idea.md"))
                .map(|e| e.id.clone())
                .unwrap()
        };
        assert_eq!(id_of(&first), id_of(&again));
    }

    /// The shape frontmatter a person wrote actually has: some of it. Obsidian's tag control adds
    /// a block containing nothing but `tags:`, and demanding `id` and `date` meant that tagging a
    /// note in the tool the vault is advertised as being editable in *removed it from the library*.
    #[test]
    fn frontmatter_with_tags_but_no_id_is_adopted_rather_than_skipped() {
        let dir = vault();
        write(
            dir.path(),
            "tagged.md",
            "---\ntags: [khách-hàng]\ncolor: teal\n---\n# Đã gắn thẻ\n",
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();

        assert!(index.skipped().is_empty(), "{:?}", index.skipped());
        let entry = index
            .entries()
            .iter()
            .find(|e| e.path.ends_with("tagged.md"))
            .expect("a partly-filled file must list");
        assert_eq!(entry.tags, vec!["khách-hàng"]);
        assert_eq!(entry.color, Some("teal"));
        assert!(entry.id.as_str().starts_with("adopted-"));
    }

    /// The trap this walked into once: `MeetingId::default()` mints a fresh UUIDv7, so filling the
    /// gap with it looked like it worked and gave the same note a new identity on every scan.
    #[test]
    fn a_partly_filled_file_keeps_the_same_id_across_scans() {
        let dir = vault();
        write(
            dir.path(),
            "tagged.md",
            "---\ntags: [x]\n---\n# Đã gắn thẻ\n",
        );
        let id_of = |index: &MeetingIndex| {
            index
                .entries()
                .iter()
                .find(|e| e.path.ends_with("tagged.md"))
                .map(|e| e.id.clone())
                .unwrap()
        };
        assert_eq!(
            id_of(&MeetingIndex::scan(dir.path()).unwrap()),
            id_of(&MeetingIndex::scan(dir.path()).unwrap())
        );
    }

    /// A file that *does* name an id keeps it — filling must only fill what is missing.
    #[test]
    fn filling_does_not_overwrite_what_the_file_already_says() {
        let dir = vault();
        let index = MeetingIndex::scan(dir.path()).unwrap();
        assert!(
            index.entries().iter().any(|e| e.id.as_str() == "01A"),
            "an id written in the file must survive"
        );
    }

    #[test]
    fn two_hand_written_files_do_not_share_an_id() {
        assert_ne!(
            adopted_id(Path::new("notes/a.md")),
            adopted_id(Path::new("notes/b.md"))
        );
    }

    /// Listing an adopted file and then failing to open it would be the same bug one screen later.
    #[test]
    fn an_adopted_file_opens() {
        let dir = vault();
        write(dir.path(), "idea.md", "# Ý tưởng\n\nvài dòng\n");
        let index = MeetingIndex::scan(dir.path()).unwrap();
        let entry = index
            .entries()
            .iter()
            .find(|e| e.path.ends_with("idea.md"))
            .unwrap();

        let doc = load(entry).expect("an adopted file must open");
        assert_eq!(doc.title, "Ý tưởng");
        assert!(doc.body.contains("vài dòng"));
        assert_eq!(
            doc.frontmatter.id, entry.id,
            "the listing and the document must agree"
        );
    }

    /// A note written last March belongs in March. Filing it under the day the vault happened to be
    /// scanned would reorder somebody's whole library the first time they ran a new build.
    #[test]
    fn an_adopted_file_is_dated_by_its_own_modification_time() {
        let dir = vault();
        write(dir.path(), "idea.md", "# Ý tưởng\n");
        let path = dir.path().join("idea.md");
        // 2021-03-04T05:06:07Z
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_614_834_367);
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();

        let index = MeetingIndex::scan(dir.path()).unwrap();
        let entry = index
            .entries()
            .iter()
            .find(|e| e.path.ends_with("idea.md"))
            .unwrap();
        assert!(entry.date.starts_with("2021-03-04"), "{}", entry.date);
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
        assert_eq!(
            weeks.len(),
            1,
            "got {:?}",
            weeks.iter().map(|w| &w.key).collect::<Vec<_>>()
        );
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

    /// Both kinds live in the same vault and the library has always returned both. What was
    /// missing was any way to ask for one — which is why the second navigation item listed
    /// meetings and called itself the library.
    #[test]
    fn a_kind_filter_separates_recordings_from_notes() {
        let dir = vault();
        write(
            dir.path(),
            "y-tuong.md",
            "---\nid: 01N\ndate: 2026-08-06\n---\n# Ý tưởng\n\nvài dòng\n",
        );
        let index = MeetingIndex::scan(dir.path()).unwrap();

        let notes = index.filter(&Filter {
            kind: Some(Kind::Note),
            ..Default::default()
        });
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Ý tưởng");

        let recordings = index.filter(&Filter {
            kind: Some(Kind::Meeting),
            ..Default::default()
        });
        assert!(recordings.iter().all(|e| e.kind == Kind::Meeting));
        assert!(!recordings.is_empty());

        // Absent means both, which is what the workspace shows by default.
        assert_eq!(
            index.filter(&Filter::default()).len(),
            notes.len() + recordings.len()
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
        assert_eq!(
            index.len(),
            3,
            "a file in a dot-folder must stay out of the list"
        );
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
