//! The library: everything the app needs to browse meetings that already exist.
//!
//! Every request rescans the vault rather than consulting a cached listing. That is not laziness —
//! the vault is a folder a user edits in Obsidian, moves files around in, and syncs with tools we
//! do not control, so any cache we held would be wrong in exactly the situations that matter most.
//! A scan costs about 5 ms per thousand meetings (ADR 0002), which is cheaper than the invalidation
//! logic would be, and it is never stale.

use std::path::{Path, PathBuf};

use crate::{
    index::{Excerpt, Filter, MeetingEntry, MeetingIndex, Skipped, Stats, load},
    meeting::Frontmatter,
    slug::slugify,
    write::write_atomically,
};
use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, Segment, paths::Paths};
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
    /// Only what is filed nowhere: the vault root.
    ///
    /// A flag rather than `folder=`, because an empty query parameter cannot be told apart from an
    /// absent one — so the root is the single folder `folder` cannot name. It is also the one
    /// people most want to filter to, being the pile of things they have not got round to filing.
    #[serde(default)]
    pub unfiled: bool,
    /// Comma-separated, because this arrives as a query string and `tag=a&tag=b` is not something
    /// every deserialiser agrees on. All of them must match — see [`Filter::tags`].
    pub tag: Option<String>,
    pub colour: Option<String>,
    pub person: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub without_summary: bool,
}

impl LibraryQuery {
    fn filter(&self) -> Filter {
        Filter {
            // `Some("")` is the root and matches nothing below it, which is what "unfiled" means.
            folder: if self.unfiled {
                Some(String::new())
            } else {
                self.folder.clone()
            },
            tags: self
                .tag
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect(),
            colour: self.colour.clone(),
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
    /// `meeting` or `note`. The library lists both; a reader still wants to know which is which.
    pub kind: crate::index::Kind,
    pub title: String,
    pub folder: String,
    pub date: String,
    pub day: String,
    pub duration: u64,
    pub participants: Vec<String>,
    pub tags: Vec<String>,
    /// A palette name, or nothing. Never a colour the client has to sanitise — [`crate::colour`].
    pub color: Option<&'static str>,
    pub has_summary: bool,
    pub size_bytes: u64,
    pub file: String,
}

impl MeetingSummary {
    fn new(entry: &MeetingEntry, meetings_root: &Path) -> Self {
        Self {
            id: entry.id.clone(),
            kind: entry.kind,
            title: entry.title.clone(),
            folder: entry.folder.clone(),
            date: entry.date.clone(),
            day: entry.day.clone(),
            duration: entry.duration,
            participants: entry.participants.iter().map(|p| unlink(p)).collect(),
            tags: entry.tags.clone(),
            color: entry.color,
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
    /// Colours in use, with counts. Only what is actually on something — see
    /// [`MeetingIndex::colours`](crate::index::MeetingIndex::colours).
    pub colours: Vec<ColourCount>,
    /// Every colour that *can* be set, in picker order.
    ///
    /// Sent rather than hardcoded in the client so the palette has one definition. A second copy in
    /// TypeScript would be a second copy to keep in step with the theme, and the failure mode is a
    /// swatch the user can pick and the daemon then refuses.
    pub palette: Vec<&'static str>,
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
pub struct ColourCount {
    pub name: &'static str,
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
    /// The heading a person reads. The draft marker is storage, not content, so it is stripped
    /// here and reported as `draft` instead — a client that had to know about HTML comments to
    /// render a heading would be a client that eventually forgets to.
    pub heading: String,
    pub body: String,
    /// Written by the agent and not yet approved. See `crate::pending`.
    #[serde(default)]
    pub draft: bool,
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

    /// Scan the vault — recordings *and* typed notes.
    ///
    /// Both, because every question the library answers is a question about both: searching for
    /// "ngân sách" should find the note somebody typed about it as readily as the meeting where it
    /// was said. They are the same documents; only the filing differs.
    ///
    /// Callers that need several views of one scan should call this once.
    pub fn scan(&self) -> Result<MeetingIndex> {
        MeetingIndex::scan_all([self.root().as_path(), self.paths.notes().as_path()])
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
            colours: index
                .colours()
                .into_iter()
                .map(|(name, count)| ColourCount { name, count })
                .collect(),
            palette: crate::colour::PALETTE.iter().map(|s| s.name).collect(),
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
                    draft: crate::pending::is_draft(&s.heading),
                    heading: crate::pending::strip(&s.heading).to_string(),
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

    /// Set or clear a document's colour.
    ///
    /// `None` removes the field rather than writing `color: none`, so a note the user has finished
    /// colour-coding goes back to looking exactly like one that never was — no residue in the file
    /// to explain to somebody reading it in Obsidian.
    pub fn set_colour(&self, id: &MeetingId, colour: Option<&str>) -> Result<Option<&'static str>> {
        let index = self.scan()?;
        let entry = index
            .get(id)
            .ok_or_else(|| Error::Vault(format!("no meeting with id {}", id.as_str())))?;
        let mut doc = load(entry)?;

        let chosen = colour.map(crate::colour::parse).transpose()?;
        doc.frontmatter.color = chosen.map(str::to_string);
        write_atomically(&entry.path, doc.to_markdown()?.as_bytes())?;
        Ok(chosen)
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

    /// The claim "Summo is a note app" is only true if searching finds notes. Before the index
    /// scanned both trees, a note somebody typed was invisible to every question they could ask.
    #[test]
    fn searching_finds_a_typed_note_as_readily_as_a_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());

        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(
            paths.meetings().join("hop.md"),
            "---\nid: 01A\ndate: 2026-08-10\nduration: 600\n---\n\n# Họp\n\n## Transcript\n**[00:00:00] Ngọc** — chốt ngân sách quý bốn <!-- seq:0 end:2.00 -->\n",
        )
        .unwrap();

        crate::note::create(&paths, "Ý tưởng", "2026-08-11", "ngân sách năm sau thì sao").unwrap();

        let library = Library::new(paths);
        let hits = library.search("ngân sách", 10).unwrap();

        let titles: Vec<&str> = hits.iter().map(|h| h.meeting.title.as_str()).collect();
        assert!(titles.contains(&"Họp"), "{titles:?}");
        assert!(
            titles.contains(&"Ý tưởng"),
            "the note is findable too: {titles:?}"
        );
    }

    /// Same document, different kind — and the kind comes from the transcript, so a screen that
    /// wants only recordings can still have them.
    #[test]
    fn the_index_can_tell_a_note_from_a_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());

        std::fs::create_dir_all(paths.meetings()).unwrap();
        std::fs::write(
            paths.meetings().join("hop.md"),
            "---\nid: 01A\ndate: 2026-08-10\nduration: 600\n---\n\n# Họp\n\n## Transcript\n",
        )
        .unwrap();
        crate::note::create(&paths, "Ghi chú", "2026-08-11", "x").unwrap();

        let index = Library::new(paths).scan().unwrap();
        assert_eq!(index.meetings().count(), 1);
        assert_eq!(index.notes().count(), 1);
        assert_eq!(index.entries().len(), 2);
    }

    /// A vault with no notes yet is the normal state of a fresh install.
    #[test]
    fn a_missing_notes_folder_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();

        assert!(Library::new(paths).scan().unwrap().entries().is_empty());
    }
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

    /// The whole promise of ADR 0006, as a test: somebody types a tag and a colour into a file in
    /// Obsidian, and Summo sees both on the next scan. No import, no sync step, no database row.
    #[test]
    fn a_tag_and_a_colour_typed_by_hand_need_no_import() {
        let (dir, lib) = library();
        // No `id`, no `date`. That is what Obsidian's own tag control writes, and what anybody
        // adding a colour by hand writes — a ULID is not a thing a person has. The first version
        // of this test supplied both, which made it pass against a file Summo itself would have
        // written and proved nothing about the case it was named for.
        fs::write(
            lib.paths.meetings().join("bang-tay.md"),
            "---\ntags: [khách-hàng, hợp-đồng]\ncolor: \"#0f7350\"\n---\n# Viết tay\n",
        )
        .unwrap();
        // Nothing between writing the file and asking for the library.
        let view = lib.view(&LibraryQuery::default(), now()).unwrap();
        drop(dir);

        let listed = view
            .groups
            .iter()
            .flat_map(|g| &g.meetings)
            .find(|m| m.title == "Viết tay")
            .expect("a hand-written file must list");
        assert_eq!(listed.tags, vec!["khách-hàng", "hợp-đồng"]);
        assert_eq!(
            listed.color,
            Some("green"),
            "a hex somebody typed becomes a palette colour rather than an error"
        );
        assert!(
            view.colours
                .iter()
                .any(|c| c.name == "green" && c.count == 1)
        );
    }

    /// Narrowing is what a finder is for: one tag is a hundred notes, two is the four you wanted.
    #[test]
    fn filtering_by_two_tags_wants_both_of_them() {
        let (_dir, lib) = library();
        fs::write(
            lib.paths.meetings().join("ca-hai.md"),
            "---\nid: 01Y\ndate: 2026-08-11T09:00:00+07:00\ntags: [weekly, sales]\n---\n# Cả hai\n",
        )
        .unwrap();

        let both = |tag: &str| {
            lib.view(
                &LibraryQuery {
                    tag: Some(tag.to_string()),
                    ..Default::default()
                },
                now(),
            )
            .unwrap()
            .total
        };
        assert_eq!(both("weekly"), 2, "the new file and the weekly sync");
        assert_eq!(both("weekly,sales"), 1, "only the one carrying both");
        assert_eq!(
            both("weekly, sales"),
            1,
            "spaces after the comma are typing, not a tag"
        );
    }

    #[test]
    fn a_colour_is_set_cleared_and_filtered_by() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());

        assert_eq!(lib.set_colour(&id, Some("teal")).unwrap(), Some("teal"));
        let filtered = lib
            .view(
                &LibraryQuery {
                    colour: Some("teal".into()),
                    ..Default::default()
                },
                now(),
            )
            .unwrap();
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.groups[0].meetings[0].title, "Weekly Sync");

        assert_eq!(lib.set_colour(&id, None).unwrap(), None);
        assert!(lib.scan().unwrap().get(&id).unwrap().color.is_none());
    }

    /// Clearing must leave a file that looks like one which was never coloured, because somebody is
    /// going to read it in Obsidian and `color: none` would need explaining.
    #[test]
    fn clearing_a_colour_removes_the_field_rather_than_emptying_it() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        lib.set_colour(&id, Some("pink")).unwrap();
        lib.set_colour(&id, None).unwrap();

        let path = lib.scan().unwrap().get(&id).unwrap().path.clone();
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("color"), "the field must be gone:\n{text}");
    }

    /// The app has a picker, so anything else arriving is a caller's bug worth naming.
    #[test]
    fn a_colour_outside_the_palette_is_refused_and_nothing_is_written() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let before =
            fs::read_to_string(lib.scan().unwrap().get(&id).unwrap().path.clone()).unwrap();

        assert!(lib.set_colour(&id, Some("chartreuse")).is_err());
        let after = fs::read_to_string(lib.scan().unwrap().get(&id).unwrap().path.clone()).unwrap();
        assert_eq!(
            before, after,
            "a refused colour must not have touched the file"
        );
    }

    /// Asking "which are green?" when nothing understands green must answer nothing, not
    /// everything-without-a-colour.
    #[test]
    fn filtering_by_a_colour_that_is_not_one_matches_nothing() {
        let (_dir, lib) = library();
        let view = lib
            .view(
                &LibraryQuery {
                    colour: Some("}, body { display: none } .x {".into()),
                    ..Default::default()
                },
                now(),
            )
            .unwrap();
        assert_eq!(view.total, 0);
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
        assert!(
            lib.rename(&MeetingId::from("01A".to_string()), "  ")
                .is_err()
        );
    }

    #[test]
    fn deleting_moves_to_trash_rather_than_unlinking() {
        let (_dir, lib) = library();
        let id = MeetingId::from("01A".to_string());
        let target = lib.trash(&id).unwrap();

        assert!(target.exists(), "the file must still exist in the trash");
        assert!(lib.scan().unwrap().get(&id).is_none());
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("Weekly Sync"),
            "the trashed file must still be the meeting"
        );
    }
}
