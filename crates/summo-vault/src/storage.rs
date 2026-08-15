//! What Summo is using on disk, and getting it back.
//!
//! Audio dominates everything else by two orders of magnitude — a transcript is a few kilobytes and
//! its recording a few megabytes — so retention is really a question about audio alone. Deleting a
//! recording after thirty days while keeping the transcript forever is the default because that
//! matches what the recording is *for*: checking a line the model may have got wrong, which people
//! do within days of a meeting and almost never a year later.
//!
//! Nothing here deletes a transcript. Reclaiming space must never cost the record of what was said.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use summo_core::{Error, MeetingId, Result, paths::Paths};

use crate::index::MeetingIndex;

/// Disk used, broken down the way a person would ask about it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Usage {
    /// Markdown: meetings, notes, people.
    pub vault_bytes: u64,
    /// Recordings.
    pub audio_bytes: u64,
    /// Downloaded model blobs, which are shared between meetings and are not per-meeting cost.
    pub model_bytes: u64,
    pub total_bytes: u64,
    /// Meetings whose audio is still on disk, largest first.
    pub recordings: Vec<Recording>,
    /// Audio directories with no meeting left to explain them.
    pub orphaned: Vec<Recording>,
}

/// One meeting's audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recording {
    pub id: MeetingId,
    pub title: String,
    /// `YYYY-MM-DD`, empty when the meeting is gone and only its audio remains.
    pub day: String,
    pub bytes: u64,
    pub files: usize,
    pub path: PathBuf,
}

/// What a prune did, or would do.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Pruned {
    pub removed: Vec<Recording>,
    /// Pictures no document links to any more, by vault-relative path.
    ///
    /// Separate from `removed`, which is recordings, because they answer different questions: a
    /// recording is deleted for being *old* and an attachment for being *unreferenced*, and a
    /// person reading the result should be able to see which is which.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    pub freed_bytes: u64,
    /// True when nothing was actually deleted.
    pub dry_run: bool,
}

/// Measure everything.
pub fn usage(paths: &Paths) -> Result<Usage> {
    let index = MeetingIndex::scan(paths.meetings())?;
    let by_id: BTreeMap<&str, &crate::index::MeetingEntry> =
        index.entries().iter().map(|e| (e.id.as_str(), e)).collect();

    let mut recordings = Vec::new();
    let mut orphaned = Vec::new();
    let audio_root = paths.audio();

    for entry in std::fs::read_dir(&audio_root)
        .into_iter()
        .flatten()
        .flatten()
    {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        let (bytes, files) = dir_size(&path);
        if files == 0 {
            continue;
        }

        let recording = match by_id.get(id.as_str()) {
            Some(meeting) => Recording {
                id: MeetingId::from(id),
                title: meeting.title.clone(),
                day: meeting.day.clone(),
                bytes,
                files,
                path,
            },
            None => Recording {
                id: MeetingId::from(id),
                title: String::new(),
                day: String::new(),
                bytes,
                files,
                path,
            },
        };
        if recording.title.is_empty() {
            orphaned.push(recording);
        } else {
            recordings.push(recording);
        }
    }

    recordings.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.id.as_str().cmp(b.id.as_str())));
    orphaned.sort_by_key(|r| std::cmp::Reverse(r.bytes));

    let vault_bytes = dir_size(&paths.vault()).0;
    let audio_bytes = recordings.iter().chain(&orphaned).map(|r| r.bytes).sum();
    let model_bytes = dir_size(&paths.models()).0;

    Ok(Usage {
        vault_bytes,
        audio_bytes,
        model_bytes,
        total_bytes: vault_bytes + audio_bytes + model_bytes,
        recordings,
        orphaned,
    })
}

/// Delete recordings older than `retention_days`, keeping every transcript.
///
/// `retention_days == 0` means keep audio forever, which is why it is the value that does nothing
/// rather than the value that deletes everything — a setting that erases a year of recordings when
/// somebody clears a text field would be indefensible.
///
/// Orphaned audio is removed regardless of age: the meeting it belonged to is already in the trash,
/// and its recording is the last thing holding the space.
///
/// Pictures nobody links to go the same way, and for the same reason. A note that loses its only
/// screenshot leaves the file behind, and a folder somebody syncs that only ever grows is one they
/// eventually delete wholesale. It happens *here* rather than on save because the question is a
/// whole-vault one: the same picture can be linked from a note nobody has opened this year, and a
/// sweep that only looked at the note being written would delete it.
pub fn prune(paths: &Paths, retention_days: u32, today: &str, dry_run: bool) -> Result<Pruned> {
    let usage = usage(paths)?;
    let mut removed = Vec::new();

    if retention_days > 0 {
        for recording in usage.recordings {
            let Some(age) = days_between(&recording.day, today) else {
                continue;
            };
            if age > i64::from(retention_days) {
                removed.push(recording);
            }
        }
    }
    removed.extend(usage.orphaned);

    let orphans = crate::attachment::unreferenced(paths, &linked_pictures(paths)?)?;
    let mut freed_bytes: u64 = removed.iter().map(|r| r.bytes).sum();
    for orphan in &orphans {
        freed_bytes += std::fs::metadata(orphan).map(|m| m.len()).unwrap_or(0);
    }

    if !dry_run {
        for recording in &removed {
            std::fs::remove_dir_all(&recording.path).map_err(|e| Error::io(&recording.path, e))?;
        }
        for orphan in &orphans {
            std::fs::remove_file(orphan).map_err(|e| Error::io(orphan, e))?;
        }
    }

    Ok(Pruned {
        removed,
        attachments: orphans
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| format!("attachments/{}", name.to_string_lossy()))
            .collect(),
        freed_bytes,
        dry_run,
    })
}

/// Every picture any document in the vault points at.
///
/// Read from the Markdown rather than from a table of references, because the Markdown is the only
/// thing that is true: a person can add `![x](attachments/y.png)` to a note in Obsidian, and a
/// reference count Summo kept would not know. Whole files, not the listing's head — a picture in the
/// last paragraph of a long meeting is exactly the one a truncated read would miss and delete.
fn linked_pictures(paths: &Paths) -> Result<Vec<String>> {
    let mut links = Vec::new();
    for root in [paths.meetings(), paths.notes(), paths.people()] {
        for entry in MeetingIndex::scan(&root)?.entries() {
            let Ok(text) = std::fs::read_to_string(&entry.path) else {
                continue;
            };
            links.extend(crate::attachment::links_in(&text));
        }
    }
    // The voice book is JSON rather than Markdown and is where a person's photograph is named. Read
    // as text for the same reason the notes are: what matters is that the path appears at all, not
    // what shape of document it appears in. Nothing in here is Summo-named today — an avatar keeps
    // whatever the user called it, and this sweep only ever deletes files it minted itself — but
    // that is a fact about the avatar screen, not a promise, and one uploaded through the picture
    // route would otherwise be deleted for looking unused.
    if let Ok(book) = std::fs::read_to_string(paths.voices().join("book.json")) {
        links.extend(crate::attachment::links_in(&book));
    }
    links.sort();
    links.dedup();
    Ok(links)
}

/// Delete one meeting's audio, keeping its transcript.
pub fn forget_audio(paths: &Paths, id: &MeetingId) -> Result<u64> {
    let dir = paths.audio_for(id);
    if !dir.exists() {
        return Ok(0);
    }
    let (bytes, _) = dir_size(&dir);
    std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    Ok(bytes)
}

/// Bytes and file count under a directory.
fn dir_size(root: &Path) -> (u64, usize) {
    let mut bytes = 0;
    let mut files = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => {
                    if let Ok(meta) = entry.metadata() {
                        bytes += meta.len();
                        files += 1;
                    }
                }
                _ => {}
            }
        }
    }
    (bytes, files)
}

/// Whole days from `from` to `to`, both `YYYY-MM-DD`.
fn days_between(from: &str, to: &str) -> Option<i64> {
    let parse = |day: &str| -> Option<i64> {
        let mut parts = day.split('-');
        let y: i32 = parts.next()?.parse().ok()?;
        let m: u8 = parts.next()?.parse().ok()?;
        let d: u8 = parts.next()?.parse().ok()?;
        let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, d).ok()?;
        Some(date.to_julian_day().into())
    };
    Some(parse(to)? - parse(from)?)
}

/// Bytes as something a person reads: `5.4 MB`, not `5662310`.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Paths) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        paths.ensure().unwrap();
        (dir, paths)
    }

    fn meeting(paths: &Paths, id: &str, day: &str, audio_bytes: usize) {
        fs::write(
            paths.meetings().join(format!("{id}.md")),
            format!("---\nid: {id}\ndate: {day}T10:00:00+07:00\nduration: 600\n---\n# Họp {id}\n"),
        )
        .unwrap();
        if audio_bytes > 0 {
            let dir = paths.audio_for(&MeetingId::from(id.to_string()));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("mic.opus"), vec![0u8; audio_bytes]).unwrap();
        }
    }

    #[test]
    fn usage_separates_audio_from_the_transcripts_it_belongs_to() {
        let (_dir, paths) = vault();
        meeting(&paths, "01A", "2026-08-09", 5_000);
        meeting(&paths, "01B", "2026-08-08", 2_000);

        let usage = usage(&paths).unwrap();
        assert_eq!(usage.audio_bytes, 7_000);
        assert!(usage.vault_bytes > 0 && usage.vault_bytes < 1_000);
        assert_eq!(usage.recordings.len(), 2);
        // Largest first, so the thing worth deleting is the thing on screen.
        assert_eq!(usage.recordings[0].bytes, 5_000);
        assert_eq!(usage.recordings[0].title, "Họp 01A");
    }

    /// The sweep has to read the *whole* of every document. A reference count Summo kept would not
    /// know about a picture somebody added to a note in Obsidian, and a truncated read would miss
    /// one in the last paragraph of a long meeting — which is the picture it would then delete.
    #[test]
    fn pruning_deletes_a_picture_nothing_links_to_and_spares_one_something_does() {
        let (_dir, paths) = vault();
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        let jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF";

        let kept = crate::attachment::store(&paths, png).unwrap();
        let dropped = crate::attachment::store(&paths, jpeg).unwrap();
        let padding = "x".repeat(5_000);
        crate::note::create(
            &paths,
            "Có ảnh",
            "2026-08-09",
            &format!("{padding}\n\nCuối cùng ![sơ đồ]({kept})"),
        )
        .unwrap();

        let pruned = prune(&paths, 0, "2026-08-10", false).unwrap();
        assert_eq!(pruned.attachments, [dropped.clone()].as_slice());
        assert!(
            crate::attachment::path_of(&paths, &kept).is_some(),
            "{kept}"
        );
        assert!(crate::attachment::path_of(&paths, &dropped).is_none());
    }

    /// A dry run says what it would do and does none of it. Losing a picture to a button labelled
    /// "see what this would free" is the worst version of this feature.
    #[test]
    fn a_dry_run_names_the_pictures_it_would_delete_and_deletes_none() {
        let (_dir, paths) = vault();
        let orphan = crate::attachment::store(&paths, b"GIF89a....").unwrap();

        let pruned = prune(&paths, 0, "2026-08-10", true).unwrap();
        assert_eq!(pruned.attachments, [orphan.clone()].as_slice());
        assert!(crate::attachment::path_of(&paths, &orphan).is_some());
    }

    #[test]
    fn pruning_deletes_old_audio_and_keeps_every_transcript() {
        let (_dir, paths) = vault();
        meeting(&paths, "old", "2026-06-01", 9_000);
        meeting(&paths, "new", "2026-08-09", 1_000);

        let pruned = prune(&paths, 30, "2026-08-10", false).unwrap();
        assert_eq!(pruned.removed.len(), 1);
        assert_eq!(pruned.removed[0].id.as_str(), "old");
        assert_eq!(pruned.freed_bytes, 9_000);

        assert!(
            paths.meetings().join("old.md").exists(),
            "a transcript was deleted"
        );
        assert!(
            !paths
                .audio_for(&MeetingId::from("old".to_string()))
                .exists()
        );
        assert!(
            paths
                .audio_for(&MeetingId::from("new".to_string()))
                .exists()
        );
    }

    #[test]
    fn a_dry_run_reports_without_deleting() {
        let (_dir, paths) = vault();
        meeting(&paths, "old", "2026-06-01", 9_000);

        let pruned = prune(&paths, 30, "2026-08-10", true).unwrap();
        assert_eq!(pruned.freed_bytes, 9_000);
        assert!(pruned.dry_run);
        assert!(
            paths
                .audio_for(&MeetingId::from("old".to_string()))
                .exists(),
            "a dry run deleted something"
        );
    }

    #[test]
    fn zero_days_means_keep_forever_rather_than_delete_everything() {
        // Somebody clearing a text field must not erase a year of recordings.
        let (_dir, paths) = vault();
        meeting(&paths, "ancient", "2020-01-01", 9_000);

        let pruned = prune(&paths, 0, "2026-08-10", false).unwrap();
        assert!(pruned.removed.is_empty());
        assert!(
            paths
                .audio_for(&MeetingId::from("ancient".to_string()))
                .exists()
        );
    }

    #[test]
    fn audio_on_the_retention_boundary_survives_one_more_day() {
        let (_dir, paths) = vault();
        meeting(&paths, "edge", "2026-07-11", 1_000); // exactly 30 days before
        let pruned = prune(&paths, 30, "2026-08-10", true).unwrap();
        assert!(
            pruned.removed.is_empty(),
            "deleted on the day it was still within retention"
        );

        let pruned = prune(&paths, 30, "2026-08-11", true).unwrap();
        assert_eq!(pruned.removed.len(), 1);
    }

    #[test]
    fn audio_whose_meeting_is_gone_is_reported_and_reclaimed() {
        let (_dir, paths) = vault();
        let orphan = paths.audio_for(&MeetingId::from("ghost".to_string()));
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("mic.opus"), vec![0u8; 4_000]).unwrap();

        let usage = usage(&paths).unwrap();
        assert_eq!(usage.orphaned.len(), 1);
        assert_eq!(usage.audio_bytes, 4_000);

        // Age does not apply: the meeting is already deleted, so this is pure waste.
        let pruned = prune(&paths, 0, "2026-08-10", false).unwrap();
        assert_eq!(pruned.freed_bytes, 4_000);
        assert!(!orphan.exists());
    }

    #[test]
    fn one_meetings_audio_can_be_dropped_on_request() {
        let (_dir, paths) = vault();
        meeting(&paths, "01A", "2026-08-09", 3_000);
        let id = MeetingId::from("01A".to_string());

        assert_eq!(forget_audio(&paths, &id).unwrap(), 3_000);
        assert!(paths.meetings().join("01A.md").exists());
        // Asking twice is not an error.
        assert_eq!(forget_audio(&paths, &id).unwrap(), 0);
    }

    #[test]
    fn sizes_read_as_sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(5_662_310), "5.4 MB");
        assert_eq!(human_bytes(120 * 1024 * 1024), "120 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
