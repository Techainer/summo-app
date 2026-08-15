//! The vault: meetings and notes as Markdown files the user owns.
//!
//! Summo's durable state is a folder of Markdown, not a database. That is a deliberate constraint
//! rather than a simplification:
//!
//! * The user can open the same folder in Obsidian, grep it, diff it, or sync it with whatever they
//!   already use, without asking us for an export.
//! * Anything derived — the search index, embeddings, the meeting list — can be deleted and rebuilt
//!   from these files, so a corrupt index is never a data-loss event.
//! * A file that Summo did not write is still readable, so a user can fix a transcript by hand or
//!   write notes between sessions and the app picks them up.
//!
//! The cost is that parsing must be forgiving: these files *will* be edited by other tools.

pub mod annotate;
pub mod attachment;
pub mod colour;
pub mod export;
pub mod index;
pub mod library;
pub mod meeting;
pub mod note;
pub mod pending;
pub mod report;
pub mod slug;
pub mod storage;
pub mod tasks;
pub mod tasks_io;
pub mod template;
pub mod translation;
pub mod write;

pub use export::{Format, Options as ExportOptions, export};
pub use index::{Filter, Group, Hit, MeetingEntry, MeetingIndex, Stats, fold, open};
pub use library::{Library, LibraryQuery, LibraryView, MeetingDetail, MeetingSummary};
pub use meeting::{Frontmatter, MeetingDoc};
pub use slug::{meeting_stem, slugify};
pub use storage::{Pruned, Usage, human_bytes};
pub use write::write_atomically;
