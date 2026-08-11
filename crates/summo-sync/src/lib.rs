//! Keeping two machines' vaults the same, without the relay being able to read them.
//!
//! The vault is a folder of Markdown files (ADR 0002), so sync is file sync — not database
//! replication, and not an operational-transform log. That choice is forced rather than chosen: a
//! user can edit the vault in Obsidian, in vim, from a script, or by restoring a folder from
//! backup, and none of those go through Summo. Any design that assumes every change passes through
//! the application is wrong here on the first day somebody opens Obsidian.
//!
//! ```text
//!   scan ──► plan(local, remote, base) ──► merge ──► seal ──► push
//!    │            │                          │        │
//!    │            │                          │        └─ the relay stores bytes it cannot read
//!    │            │                          └─ three-way, or a conflict copy beside the original
//!    │            └─ base is what the two sides agreed on last time
//!    └─ content hashes, not timestamps
//! ```
//!
//! ## Why not CRDTs
//!
//! They are the right answer when every edit is observed, and the wrong one here for the same
//! reason: a CRDT's guarantees come from seeing operations, and this vault receives whole files
//! from editors that never heard of Summo. A CRDT fed a rewritten file has to diff it back into
//! operations anyway — at which point it is a three-way merge with more machinery, more state to
//! keep in sync, and a failure mode nobody can inspect. Obsidian Sync and Syncthing both landed on
//! files-and-conflict-copies, and they landed there from the same constraint.
//!
//! ## What the relay learns
//!
//! Names and contents are both encrypted ([`crypto`]). The relay sees a set of opaque ids, their
//! sizes, and when they changed. That is not nothing — sizes and timing are metadata — but it is
//! the floor for a service that has to store and address the blobs at all, and it means a breach of
//! the relay is not a breach of anybody's meetings.
//!
//! The key never leaves the machine. Losing it means losing the ability to read what was uploaded,
//! which is the correct trade for a product whose entire claim is that your recordings are yours.

pub mod crypto;
pub mod merge;
pub mod plan;
pub mod remote;
pub mod run;
pub mod snapshot;

pub use crypto::{Key, Sealed};
pub use merge::{Merged, merge};
pub use plan::{Action, Plan, Side, Summary, plan};
pub use remote::{Manifest, Remote};
pub use run::{Outcome, sync};
pub use snapshot::Snapshot;
