//! The library: everything the app needs to browse meetings that already exist.
//!
//! Every request rescans the vault rather than consulting a cached listing. That is not laziness —
//! the vault is a folder a user edits in Obsidian, moves files around in, and syncs with tools we
//! do not control, so any cache we held would be wrong in exactly the situations that matter most.
//! A scan costs about 5 ms per thousand meetings (ADR 0002), which is cheaper than the invalidation
//! logic would be, and it is never stale.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, Segment, paths::Paths};
use crate::{
    index::{Excerpt, Filter, MeetingEntry, MeetingIndex, Skipped, Stats, load},
    meeting::Frontmatter,
    slug::slugify,
    write::write_atomically,
};
use time::OffsetDateTime;

/// How meetings are grouped in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    #[default]
    Day,
    Week,
    Folder,
    /// One flat list, newest first.
    None,
}

/// What the library view asked for.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryQuery {
    /// The daemon token. It rides on the same query string as the filters because a query string
    /// can only be deserialised into one type, and splitting it would mean two structs describing
    /// one request.
    pub token: Option<String>,
    #[serde(default)]
    pub group: GroupBy,
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub person: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub without_summary: bool,
}

impl LibraryQuery {
    fn filter(&self) -> Filter {
        Filter {
            folder: self.folder.clone(),
            tag: self.tag.clone(),
            person: self.person.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            without_summary: self.without_summary,
        }
    }
}

/// A meeting as the app lists it. Paths are relative to the vault so the UI never has to reason
/// about where the vault lives, and an absolute path never leaks into a URL.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingSummary {
    pub id: MeetingId,
    pub title: String,
    pub folder: String,
    pub date: String,
    pub day: String,
    pub duration: u64,
    pub participants: Vec<String>,
    pub tags: Vec<String>,
    pub has_summary: bool,
    pub size_bytes: u64,
    pub file: String,
}

impl MeetingSummary {
    fn new(entry: &MeetingEntry, meetings_root: &Path) -> Self {
        Self {
            id: entry.id.clone(),
            title: entry.title.clone(),
            folder: entry.folder.clone(),
            date: entry.date.clone(),
            day: entry.day.clone(),
            duration: entry.duration,
            participants: entry.participants.iter().map(|p| unlink(p)).collect(),
            tags: entry.tags.clone(),
            has_summary: entry.has_summary,
            size_bytes: entry.size_bytes,
            file: relative(&entry.path, meetings_root),
        }
    }
}

/// A named group of meetings, in display order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SummaryGroup {
    pub key: String,
    pub meetings: Vec<MeetingSummary>,
}

/// Everything the library screen renders in one response.
///
/// One payload rather than five endpoints: the sidebar, the counters and the list all describe the
/// same scan, and fetching them separately would let them disagree on screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryView {
    pub groups: Vec<SummaryGroup>,
    pub total: usize,
    pub stats: Stats,
    pub folders: Vec<String>,
    pub tags: Vec<TagCount>,
    pub people: Vec<PersonCount>,
    /// Files that would not parse, so a user can find and fix them instead of wondering where a
    /// meeting went.
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PersonCount {
    pub name: String,
    pub count: usize,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub meeting: MeetingSummary,
    pub matches: usize,
    pub excerpts: Vec<Excerpt>,
}

/// A whole meeting, for the detail view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingDetail {
    pub summary: MeetingSummary,
    pub frontmatter: Frontmatter,
    pub sections: Vec<SectionView>,
    pub transcript: Vec<Segment>,
    /// Recorded audio for this meeting, if it has not been pruned by the retention setting.
    pub audio: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionView {
    pub heading: String,
    pub body: String,
}

/// Read-only access to the vault.
#[derive(Debug, Clone)]
pub struct Library {
    paths: Paths,
}

impl Library {
    #[must_use]
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    fn root(&self) -> PathBuf {
        self.paths.meetings()
    }

    /// Scan the vault. Callers that need several views of one scan should call this once.
    pub fn scan(&self) -> Result<MeetingIndex> {
        MeetingIndex::scan(self.root())
    }

    /// The library screen.
    pub fn view(&self, query: &LibraryQuery, now: OffsetDateTime) -> Result<LibraryView> {
        let index = self.scan()?;
        let root = self.root();
        let filter = query.filter();

        // Filters narrow what is listed, not what the counters count: a person filtering by tag
        // still wants to see the other tags they could switch to.
        let groups = match query.group {
            GroupBy::Day => grouped(index.by_day(), &filter, &root),
            GroupBy::Week => grouped(index.by_week(), &filter, &root),
            GroupBy::Folder => grouped(index.by_folder(), &filter, &root),
            GroupBy::None => {
                let meetings: Vec<MeetingSummary> = index
                    .filter(&filter)
                    .into_iter()
                    .map(|e| MeetingSummary::new(e, &root))
                    .collect();
                if meetings.is_empty() {
                    Vec::new()
                } else {
                    vec![SummaryGroup {
                        key: String::new(),
                        meetings,
                    }]
                }
            }
        };

        Ok(LibraryView {
            total: groups.iter().map(|g| g.meetings.len()).sum(),
            groups,
            stats: index.stats(now),
            folders: index.folders(),
            tags: index
                .tags()
                .into_iter()
                .map(|(name, count)| TagCount {
                    name: name.to_string(),
                    count,
                })
                .collect(),
            people: index
                .people()
                .into_iter()
                .map(|(name, count)| PersonCount { name, count })
                .collect(),
            skipped: index.skipped().to_vec(),
        })
    }

    /// Search the whole vault.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let index = self.scan()?;
        let root = self.root();
        Ok(index
            .search(query, limit)?
            .into_iter()
            .map(|hit| SearchHit {
                meeting: MeetingSummary::new(hit.meeting, &root),
                matches: hit.matches,
                excerpts: hit.excerpts,
            })
            .collect())
    }

    /// One meeting in full.
    pub fn detail(&self, id: &MeetingId) -> Result<MeetingDetail> {
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;
        let doc = load(entry)?;

        let audio_dir = self.paths.audio_for(id);
        let mut audio: Vec<String> = std::fs::read_dir(&audio_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        audio.sort();

        Ok(MeetingDetail {
            summary: MeetingSummary::new(entry, &self.root()),
            frontmatter: doc.frontmatter,
            sections: doc
                .sections
                .into_iter()
                .map(|s| SectionView {
                    heading: s.heading,
                    body: s.body,
                })
                .collect(),
            transcript: doc.transcript,
            audio,
        })
    }

    /// Move a meeting into a folder under `meetings/`, creating it if needed.
    ///
    /// An empty folder means the root. The file keeps its name, so links and the audio directory —
    /// which is keyed by id, not by path — stay valid.
    pub fn move_to_folder(&self, id: &MeetingId, folder: &str) -> Result<PathBuf> {
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;

        let root = self.root();
        let dir = if folder.trim().is_empty() {
            root.clone()
        } else {
            root.join(safe_folder(folder)?)
        };
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

        let name = entry
            .path
            .file_name()
            .ok_or_else(|| Error::Vault("meeting has no file name".into()))?;
        let target = dir.join(name);
        if target == entry.path {
            return Ok(target);
        }
        if target.exists() {
            return Err(Error::Vault(format!(
                "a file named {} already exists in that folder",
                name.to_string_lossy()
            )));
        }
        std::fs::rename(&entry.path, &target).map_err(|e| Error::io(&target, e))?;
        Ok(target)
    }

    /// Replace a meeting's tags.
    pub fn set_tags(&self, id: &MeetingId, tags: Vec<String>) -> Result<Vec<String>> {
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;
        let mut doc = load(entry)?;

        let mut tags: Vec<String> = tags
            .into_iter()
            .map(|t| t.trim().trim_start_matches('#').to_string())
            .filter(|t| !t.is_empty())
            .collect();
        tags.sort();
        tags.dedup();

        doc.frontmatter.tags.clone_from(&tags);
        write_atomically(&entry.path, doc.to_markdown()?.as_bytes())?;
        Ok(tags)
    }

    /// Rename a meeting. The file keeps its name: renaming it would break links from other notes,
    /// and the title in the document is what the app displays anyway.
    pub fn rename(&self, id: &MeetingId, title: &str) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Vault("a meeting needs a title".into()));
        }
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;
        let mut doc = load(entry)?;
        doc.title = title.to_string();
        write_atomically(&entry.path, doc.to_markdown()?.as_bytes())?;
        Ok(title.to_string())
    }

    /// Move a meeting and its audio to the vault's trash folder.
    ///
    /// Nothing is unlinked. A transcript is often the only record of a conversation that happened
    /// once, and the cost of keeping a file the user meant to discard is a few kilobytes, while the
    /// cost of deleting one they did not is the whole meeting. `.trash` starts with a dot, so the
    /// scan already skips it.
    pub fn trash(&self, id: &MeetingId) -> Result<PathBuf> {
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;

        let trash = self.paths.vault().join(".trash");
        std::fs::create_dir_all(&trash).map_err(|e| Error::io(&trash, e))?;

        let name = entry
            .path
            .file_name()
            .ok_or_else(|| Error::Vault("meeting has no file name".into()))?;
        let mut target = trash.join(name);
        // Two meetings on the same day with the same title collide; keep both.
        let mut n = 1;
        while target.exists() {
            target = trash.join(format!(
                "{}-{n}.md",
                Path::new(name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
            n += 1;
        }
        std::fs::rename(&entry.path, &target).map_err(|e| Error::io(&target, e))?;

        let audio = self.paths.audio_for(id);
        if audio.exists() {
            let audio_target = trash.join(format!("audio-{}", id.as_str()));
            let _ = std::fs::rename(&audio, &audio_target);
        }
        Ok(target)
    }
}

fn grouped(
    groups: Vec<crate::index::Group<'_>>,
    filter: &Filter,
    root: &Path,
) -> Vec<SummaryGroup> {
    groups
        .into_iter()
        .filter_map(|g| {
            let meetings: Vec<MeetingSummary> = g
                .meetings
                .into_iter()
                .filter(|e| filter.matches(e))
                .map(|e| MeetingSummary::new(e, root))
                .collect();
            // A day whose only meeting was filtered out is not an empty day worth a heading.
            (!meetings.is_empty()).then_some(SummaryGroup {
                key: g.key,
                meetings,
            })
        })
        .collect()
}

/// Reject anything that would escape `meetings/`.
///
/// A folder name arrives from the app, and the app takes it from a text field. `..` in that field
/// must not become a write outside the vault.
fn safe_folder(folder: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in folder.split(['/', '\\']) {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.starts_with('.') {
            return Err(Error::Vault(format!(
                "folder name {part:?} is not allowed inside the vault"
            )));
        }
        out.push(slugify(part));
    }
    if out.as_os_str().is_empty() {
        return Err(Error::Vault("folder name is empty".into()));
    }
    Ok(out)
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn unlink(text: &str) -> String {
    let t = text.trim();
    let inner = t
        .strip_prefix("[[")
        .and_then(|r| r.strip_suffix("]]"))
        .unwrap_or(t);
    inner.split('|').next().unwrap_or(inner).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn library() -> (TempDir, Library) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        paths.ensure().unwrap();

        let write = |rel: &str, id: &str, date: &str, title: &str, tags: &str| {
            let path = paths.meetings().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                format!(
                    "---\nid: {id}\ndate: {date}\nduration: 600\n\
                     participants: [\"[[Bạn]]\", \"[[Ngọc]]\"]\ntags: [{tags}]\n---\n\
                     # {title}\n\n## Tóm tắt\nChốt dùng Rust.\n\n## Transcript\n\
                     **[00:12:04] Bạn** — Mình họp về ngân sách nhé\n"
                ),
            )
            .unwrap();
        };
        write(
            "2026-08-09-weekly-sync.md",
            "01A",
            "2026-08-09T10:00:00+07:00",
            "Weekly Sync",
            "weekly",
        );
        write(
            "khach-hang/2026-08-05-demo.md",
            "01C",
            "2026-08-05T09:00:00+07:00",
            "Demo khách hàng",
            "sales",
        );
        (dir, Library::new(paths))
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_786_000_000).unwrap()
    }

    #[test]
    fn the_library_view_lists_grouped_by_day() {
        let (_dir, lib) = library();
        let view = lib.view(&LibraryQuery::default(), now()).unwrap();

        assert_eq!(view.total, 2);
        assert_eq!(view.groups.len(), 2);
        assert_eq!(view.groups[0].key, "2026-08-09");
        assert_eq!(view.groups[0].meetings[0].title, "Weekly Sync");
        assert_eq!(view.stats.meetings, 2);
    }

    #[test]
    fn listed_paths_are_relative_to_the_vault() {
        let (_dir, lib) = library();
        let view = lib.view(&LibraryQuery::default(), now()).unwrap();
        let files: Vec<&str> = view
            .groups
            .iter()
            .flat_map(|g| g.meetings.iter().map(|m| m.file.as_str()))
            .collect();

        assert!(
            files.iter().all(|f| !f.starts_with('/')),
            "an absolute path leaked into the API: {files:?}"
        );
        assert!(files.contains(&"khach-hang/2026-08-05-demo.md"));
    }

    #[test]
    fn filtering_empties_a_group_rather_than_showing_an_empty_heading() {
        let (_dir, lib) = library();
        let view = lib
            .view(
                &LibraryQuery {
                    tag: Some("sales".into()),
                    ..Default::default()
                },
                now(),
            )
            .unwrap();

        assert_eq!(view.groups.len(), 1);
        assert_eq!(view.groups[0].meetings[0].title, "Demo khách hàng");
        // The counters still describe the whole vault, so the other tag is still offerable.
        assert_eq!(view.tags.len(), 2);
    }

    #[test]
    fn search_returns_the_line_and_where_it_was_said() {
        let (_dir, lib) = library();
        let hits = lib.search("ngan sach", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].excerpts[0].t0, Some(724.0));
    }

    #[test]
    fn detail_carries_the_transcript_and_the_sections() {
        let (_dir, lib) = library();
        let detail = lib.detail(&MeetingId::from("01A".to_string())).unwrap();
        assert_eq!(detail.transcript.len(), 1);
        assert_eq!(detail.sections[0].heading, "Tóm tắt");
        assert!(detail.audio.is_empty());
    }

    #[test]
    fn an_unknown_id_is_an_error_not_an_empty_meeting() {
        let (_dir, lib) = library();
        assert!(lib.detail(&MeetingId::from("nope".to_string())).is_err());
    }

    #[test]
    fn a_meeting_moves_between_folders() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        lib.move_to_folder(&id, "khach-hang/acme").unwrap();

        let view = lib.view(&LibraryQuery::default(), now()).unwrap();
        let moved = view.groups[0].meetings[0].clone();
        assert_eq!(moved.folder, "khach-hang/acme");
        // Moving must not lose the document.
        assert_eq!(lib.detail(&id).unwrap().transcript.len(), 1);
    }

    #[test]
    fn moving_to_the_root_is_allowed() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01C".to_string());
        lib.move_to_folder(&id, "").unwrap();
        let index = lib.scan().unwrap();
        assert_eq!(index.get(&id).unwrap().folder, "");
    }

    #[test]
    fn a_folder_name_cannot_escape_the_vault() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let err = lib.move_to_folder(&id, "../../etc").unwrap_err();
        assert!(err.to_string().contains("not allowed"), "got: {err}");

        // And the file is still where it was.
        assert!(lib.scan().unwrap().get(&id).is_some());
    }

    #[test]
    fn tags_are_normalised_and_deduplicated() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let tags = lib
            .set_tags(&id, vec!["#product".into(), "product".into(), "  ".into()])
            .unwrap();
        assert_eq!(tags, vec!["product"]);
        assert_eq!(lib.scan().unwrap().get(&id).unwrap().tags, vec!["product"]);
    }

    #[test]
    fn renaming_changes_the_title_and_keeps_the_file() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let before = lib.scan().unwrap().get(&id).unwrap().path.clone();
        lib.rename(&id, "Họp sản phẩm").unwrap();

        let after = lib.scan().unwrap();
        let entry = after.get(&id).unwrap();
        assert_eq!(entry.title, "Họp sản phẩm");
        assert_eq!(entry.path, before, "renaming must not move the file");
    }

    #[test]
    fn an_empty_title_is_refused() {
        let (_dir, lib) = library();
        assert!(lib.rename(&MeetingId::from("01A".to_string()), "  ").is_err());
    }

    #[test]
    fn deleting_moves_to_trash_rather_than_unlinking() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let target = lib.trash(&id).unwrap();

        assert!(target.exists(), "the file must still exist in the trash");
        assert!(lib.scan().unwrap().get(&id).is_none());
        assert!(
            std::fs::read_to_string(&target).unwrap().contains("Weekly Sync"),
            "the trashed file must still be the meeting"
        );
    }
}
