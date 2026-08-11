//! Deciding what to do, from three snapshots.
//!
//! Two-way sync cannot tell a **new file** from a **deleted file**. If the other side has something
//! I do not, either they created it or I deleted it, and those need opposite actions. Guessing
//! wrong in one direction loses somebody's work; guessing wrong in the other resurrects a file they
//! deliberately threw away, on every machine, for ever.
//!
//! So there are three snapshots: what I have now, what they have now, and **what we agreed on last
//! time** — the base, written at the end of the previous sync. With the base, every case is a
//! lookup rather than a guess:
//!
//! ```text
//!   base   local   remote   →  what happened, and what to do
//!   ────   ─────   ──────      ──────────────────────────────
//!    —      A       —          I created it            → upload
//!    —      —       A          they created it         → download
//!    A      A       A          nothing                 → nothing
//!    A      B       A          I edited it             → upload
//!    A      A       B          they edited it          → download
//!    A      —       A          I deleted it            → delete there
//!    A      A       —          they deleted it         → delete here
//!    A      B       C          we both edited it       → merge
//!    —      A       B          we both created it      → merge
//!    A      B       —          I edited, they deleted  → keep mine, and say so
//!    A      —       B          they edited, I deleted  → keep theirs, and say so
//! ```
//!
//! The last two are the only rows where this file makes a *judgement* rather than a deduction, and
//! it is the same judgement both times: **an edit beats a delete.** Restoring a file somebody
//! deleted costs them one deletion they have to repeat. Deleting a file somebody edited costs them
//! the edit, and they may not notice for weeks. The asymmetry is not close.

use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;

/// What to do about one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum Action {
    /// Send mine; theirs is older or absent.
    Upload,
    /// Take theirs; mine is older or absent.
    Download,
    /// Both sides changed it. Contents have to be reconciled — see [`crate::merge`].
    Merge,
    /// I deleted it and they did not touch it since.
    DeleteRemote,
    /// They deleted it and I did not touch it since.
    DeleteLocal,
    /// One side deleted while the other edited. The edit wins and the file comes back.
    ///
    /// Carried as its own action rather than folded into upload or download because the user should
    /// be *told*: a file they deleted reappearing is confusing unless somebody explains why.
    Resurrect { edited_on: Side },
}

/// Which machine an edit was made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Local,
    Remote,
}

/// One decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub path: String,
    #[serde(flatten)]
    pub action: Action,
}

/// Everything to do this run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Paths the remote offered that are not safe to write. Reported, never acted on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<String>,
}

impl Plan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// How many of each kind, for the one line a user reads after a sync.
    #[must_use]
    pub fn summary(&self) -> Summary {
        let mut out = Summary::default();
        for step in &self.steps {
            match step.action {
                Action::Upload => out.uploaded += 1,
                Action::Download => out.downloaded += 1,
                Action::Merge => out.merged += 1,
                Action::DeleteRemote | Action::DeleteLocal => out.deleted += 1,
                Action::Resurrect { .. } => out.resurrected += 1,
            }
        }
        out
    }

    pub fn steps_for(&self, action: &Action) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(move |s| &s.action == action)
    }
}

/// Counts, for the line a user reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub uploaded: usize,
    pub downloaded: usize,
    pub merged: usize,
    pub deleted: usize,
    pub resurrected: usize,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} up, {} down, {} merged, {} deleted, {} restored",
            self.uploaded, self.downloaded, self.merged, self.deleted, self.resurrected
        )
    }
}

/// Work out what to do, from what I have, what they have, and what we last agreed on.
#[must_use]
pub fn plan(local: &Snapshot, remote: &Snapshot, base: &Snapshot) -> Plan {
    let mut steps = Vec::new();
    let mut refused = Vec::new();

    for path in local.paths_union(remote) {
        // A path from the remote reaches the filesystem. One that escapes the vault is refused
        // here, before anything downstream has a chance to treat it as ordinary.
        if !crate::snapshot::is_safe(&path) {
            refused.push(path);
            continue;
        }

        let mine = local.get(&path).map(|e| e.hash.as_str());
        let theirs = remote.get(&path).map(|e| e.hash.as_str());
        let agreed = base.get(&path).map(|e| e.hash.as_str());

        let Some(action) = decide(mine, theirs, agreed) else {
            continue;
        };
        steps.push(Step { path, action });
    }

    Plan { steps, refused }
}

/// The table at the top of this module, as code.
fn decide(mine: Option<&str>, theirs: Option<&str>, agreed: Option<&str>) -> Option<Action> {
    match (mine, theirs, agreed) {
        // Identical. Nothing to do, whatever the base says.
        (Some(a), Some(b), _) if a == b => None,
        (None, None, _) => None,

        // Both present and different.
        (Some(a), Some(_), Some(base)) if a == base => Some(Action::Download), // only they changed
        (Some(_), Some(b), Some(base)) if b == base => Some(Action::Upload), // only I changed
        (Some(_), Some(_), _) => Some(Action::Merge), // both changed, or both created

        // Only mine. They had it at the base and no longer do, so *they* deleted it — and the
        // copy to remove is the one still here.
        (Some(_), None, None) => Some(Action::Upload), // I created it
        (Some(a), None, Some(base)) if a == base => Some(Action::DeleteLocal), // they deleted it
        (Some(_), None, Some(_)) => Some(Action::Resurrect { edited_on: Side::Local }),

        // Only theirs. I had it at the base and no longer do, so *I* deleted it, and the copy to
        // remove is the one still on the remote.
        (None, Some(_), None) => Some(Action::Download), // they created it
        (None, Some(b), Some(base)) if b == base => Some(Action::DeleteRemote), // I deleted it
        (None, Some(_), Some(_)) => Some(Action::Resurrect { edited_on: Side::Remote }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Entry;

    fn snap(files: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            files: files
                .iter()
                .map(|(path, hash)| {
                    (
                        (*path).to_string(),
                        Entry {
                            hash: (*hash).to_string(),
                            size: 1,
                            modified: 0,
                        },
                    )
                })
                .collect(),
        }
    }

    fn action_for(local: &[(&str, &str)], remote: &[(&str, &str)], base: &[(&str, &str)]) -> Option<Action> {
        plan(&snap(local), &snap(remote), &snap(base))
            .steps
            .into_iter()
            .next()
            .map(|s| s.action)
    }

    #[test]
    fn nothing_changed_means_nothing_to_do() {
        assert_eq!(action_for(&[("a", "1")], &[("a", "1")], &[("a", "1")]), None);
    }

    #[test]
    fn two_empty_vaults_agree() {
        assert!(plan(&snap(&[]), &snap(&[]), &snap(&[])).is_empty());
    }

    #[test]
    fn a_file_only_i_have_and_never_synced_is_mine_to_send() {
        assert_eq!(action_for(&[("a", "1")], &[], &[]), Some(Action::Upload));
    }

    #[test]
    fn a_file_only_they_have_and_never_synced_is_theirs_to_send() {
        assert_eq!(action_for(&[], &[("a", "1")], &[]), Some(Action::Download));
    }

    #[test]
    fn an_edit_only_i_made_goes_up() {
        assert_eq!(
            action_for(&[("a", "2")], &[("a", "1")], &[("a", "1")]),
            Some(Action::Upload)
        );
    }

    #[test]
    fn an_edit_only_they_made_comes_down() {
        assert_eq!(
            action_for(&[("a", "1")], &[("a", "2")], &[("a", "1")]),
            Some(Action::Download)
        );
    }

    // ---- the cases two-way sync cannot tell apart -------------------------------------------

    /// Without the base this is indistinguishable from "they created it", and the file I deleted
    /// comes back on every sync for ever.
    #[test]
    fn a_file_i_deleted_is_deleted_there_rather_than_downloaded_again() {
        // Gone here, still theirs, unchanged since the base: I am the one who deleted it, so the
        // copy that has to go is the remote's. Getting this backwards deletes the wrong side's
        // file — and then the deletion never propagates, because the local copy is already gone.
        assert_eq!(
            action_for(&[], &[("a", "1")], &[("a", "1")]),
            Some(Action::DeleteRemote)
        );
    }

    #[test]
    fn a_file_they_deleted_is_deleted_here_rather_than_uploaded_again() {
        assert_eq!(
            action_for(&[("a", "1")], &[], &[("a", "1")]),
            Some(Action::DeleteLocal)
        );
    }

    // ---- the two rows that are a judgement ---------------------------------------------------

    /// Deleting a file somebody edited costs them the edit and they may not notice for weeks.
    /// Restoring one they deleted costs them one deletion they have to repeat.
    #[test]
    fn an_edit_beats_a_delete_whichever_side_edited() {
        assert_eq!(
            action_for(&[("a", "2")], &[], &[("a", "1")]),
            Some(Action::Resurrect { edited_on: Side::Local })
        );
        assert_eq!(
            action_for(&[], &[("a", "2")], &[("a", "1")]),
            Some(Action::Resurrect { edited_on: Side::Remote })
        );
    }

    // ---- both sides changed ------------------------------------------------------------------

    #[test]
    fn both_of_us_editing_the_same_file_needs_a_merge() {
        assert_eq!(
            action_for(&[("a", "2")], &[("a", "3")], &[("a", "1")]),
            Some(Action::Merge)
        );
    }

    /// Two machines that each made `notes/idea.md` before ever syncing. Neither is a base for the
    /// other, and picking one would silently drop the other.
    #[test]
    fn both_of_us_creating_the_same_path_needs_a_merge_too() {
        assert_eq!(action_for(&[("a", "2")], &[("a", "3")], &[]), Some(Action::Merge));
    }

    /// Identical contents are identical whatever the history was — two machines that happened to
    /// write the same thing have nothing to reconcile.
    #[test]
    fn the_same_contents_on_both_sides_is_never_a_conflict() {
        assert_eq!(action_for(&[("a", "1")], &[("a", "1")], &[]), None);
        assert_eq!(action_for(&[("a", "1")], &[("a", "1")], &[("a", "9")]), None);
    }

    // ---- what arrives from the other side ----------------------------------------------------

    /// The remote is a server. A traversal in its manifest must not become a write outside the
    /// vault, and it must be *reported* rather than quietly dropped.
    #[test]
    fn a_path_that_escapes_the_vault_is_refused_and_named() {
        let plan = plan(&snap(&[]), &snap(&[("../../.ssh/authorized_keys", "1")]), &snap(&[]));
        assert!(plan.is_empty(), "nothing may be done with it");
        assert_eq!(plan.refused, vec!["../../.ssh/authorized_keys"]);
    }

    #[test]
    fn a_refused_path_does_not_stop_the_rest_of_the_sync() {
        let plan = plan(
            &snap(&[]),
            &snap(&[("../evil", "1"), ("meetings/a.md", "2")]),
            &snap(&[]),
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.steps[0].path, "meetings/a.md");
        assert_eq!(plan.refused.len(), 1);
    }

    // ---- the line a user reads ---------------------------------------------------------------

    #[test]
    fn the_summary_counts_each_kind() {
        let plan = plan(
            // up:   I edited, they still have the base    → upload
            // down: they created it                       → download
            // both: we both edited                        → merge
            // mine: unchanged here, gone there            → delete here
            // rip:  I edited, they deleted                → resurrect
            &snap(&[("up", "2"), ("same", "1"), ("mine", "1"), ("both", "2"), ("rip", "2")]),
            &snap(&[("up", "1"), ("down", "1"), ("same", "1"), ("both", "3")]),
            &snap(&[("up", "1"), ("same", "1"), ("mine", "1"), ("both", "1"), ("rip", "1")]),
        );
        let summary = plan.summary();
        assert_eq!(summary.uploaded, 1, "{plan:?}");
        assert_eq!(summary.downloaded, 1, "{plan:?}");
        assert_eq!(summary.merged, 1, "{plan:?}");
        assert_eq!(summary.deleted, 1, "{plan:?}");
        assert_eq!(summary.resurrected, 1, "{plan:?}");
    }

    #[test]
    fn a_plan_is_ordered_so_two_runs_report_the_same_thing() {
        let local = snap(&[("z.md", "1"), ("a.md", "1"), ("m.md", "1")]);
        let first = plan(&local, &snap(&[]), &snap(&[]));
        let again = plan(&local, &snap(&[]), &snap(&[]));
        assert_eq!(first, again);
        assert_eq!(
            first.steps.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
            vec!["a.md", "m.md", "z.md"]
        );
    }
}
